//! Recording Studio: project sessions, editing, transport, recording,
//! import and export.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use uta_audio::graph_build::{build_graph, mix_commands, structure_key, NoPlugins, PluginFactory};
use uta_audio::recorder::RecordTarget;
use uta_audio::render::{render_to_wav, RenderOptions};
use uta_audio::source::build_sources;
use uta_audio::AudioEngine;
use uta_project::edit::{self, ClipUpdate, TrackUpdate};
use uta_project::paths::{sanitize_file_stem, PROJECT_EXT};
use uta_project::{
    new_id, Asset, AssetCheck, AssetKind, AssetStatus, EffectKind, EffectSlot, History, Id, LoadedProject, Metronome, PluginRef,
    Project, ProjectStore, Region, RevisionInfo, RevisionKind,
};

use crate::error::{CoreError, Result};
use crate::{now, AppCore};

pub const MAX_TRACKS: usize = uta_audio::shared::MAX_TRACKS;

pub(crate) struct StudioRecording {
    pub tracks: Vec<(Id, PathBuf)>,
}

pub(crate) struct StudioSession {
    pub store: ProjectStore,
    pub project: Project,
    pub history: History,
    pub dirty: bool,
    pub structure: Vec<u128>,
    /// Document state before a transient (dragging) edit began.
    pub transient_base: Option<(String, Project)>,
    pub last_autosave: Instant,
    pub asset_checks: Vec<AssetCheck>,
    pub recording: Option<StudioRecording>,
    pub warnings: Vec<String>,
    pub newer_autosave: Option<RevisionInfo>,
    /// Plugin slots whose plugin could not be loaded on this computer.
    pub missing_plugins: Vec<String>,
}

impl StudioSession {
    fn resolver(&self) -> impl Fn(&Asset) -> Option<PathBuf> + '_ {
        move |a: &Asset| self.store.asset_path(a).ok().filter(|p| p.is_file())
    }

    /// Bring the engine in line with the document. `full` forces a graph
    /// rebuild (undo/redo, project open, engine restart).
    pub fn sync_engine_with(&mut self, e: &mut AudioEngine, full: bool, plugins: &mut dyn PluginFactory) {
        if e.sample_rate() != self.project.sample_rate {
            self.warnings.retain(|w| !w.starts_with("Sample rate"));
            self.warnings.push(format!(
                "Sample rate mismatch: this project is {} Hz but the audio device runs at {} Hz. Set the device to {} Hz in Settings to play or record.",
                self.project.sample_rate,
                e.sample_rate(),
                self.project.sample_rate
            ));
        } else {
            self.warnings.retain(|w| !w.starts_with("Sample rate"));
        }
        let key = structure_key(&self.project);
        if full || key != self.structure {
            let g = build_graph(&self.project, e.sample_rate() as f32, plugins);
            if let Err(err) = e.set_graph(g) {
                tracing::error!("could not update the audio graph: {err}");
            }
            self.structure = key;
        } else {
            for c in mix_commands(&self.project) {
                let _ = e.send(c);
            }
        }
        e.set_sources(build_sources(&self.project, self.resolver()));
        let _ = e.set_loop(self.project.loop_region.map(|r| (r.start, r.end)));
        let _ = e.set_metronome(uta_audio::core::MetronomeSettings {
            enabled: self.project.metronome.enabled,
            bpm: self.project.tempo_bpm,
            beats_per_bar: self.project.time_signature.0 as u32,
            gain: uta_dsp::db_to_gain(self.project.metronome.volume_db),
        });
        let _ = e.refresh_playback();
    }

    pub fn sync_engine(&mut self, e: &mut AudioEngine, full: bool) {
        self.sync_engine_with(e, full, &mut NoPlugins);
    }

    fn sample_rate_ok(&self, e: &AudioEngine) -> Result<()> {
        if e.sample_rate() != self.project.sample_rate {
            return Err(CoreError::Invalid(format!(
                "The project is {} Hz but the audio device runs at {} Hz. Change the device sample rate in Settings.",
                self.project.sample_rate,
                e.sample_rate()
            )));
        }
        Ok(())
    }
}

/// A document edit. All positions are frames at the project sample rate.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum EditOp {
    #[serde(rename_all = "camelCase")]
    AddTrack { name: Option<String> },
    #[serde(rename_all = "camelCase")]
    RemoveTrack { track: Id },
    #[serde(rename_all = "camelCase")]
    MoveTrack { track: Id, index: usize },
    #[serde(rename_all = "camelCase")]
    UpdateTrack { track: Id, update: TrackUpdate },
    #[serde(rename_all = "camelCase")]
    MoveClip { clip: Id, position: u64, track: Option<Id> },
    #[serde(rename_all = "camelCase")]
    TrimClip { clip: Id, start: Option<u64>, end: Option<u64> },
    #[serde(rename_all = "camelCase")]
    SplitClip { clip: Id, at: u64 },
    #[serde(rename_all = "camelCase")]
    DuplicateClip { clip: Id, position: Option<u64> },
    #[serde(rename_all = "camelCase")]
    DeleteClip { clip: Id },
    #[serde(rename_all = "camelCase")]
    UpdateClip { clip: Id, update: ClipUpdate },
    #[serde(rename_all = "camelCase")]
    SetComp { track: Id, group: Id, start: u64, end: u64, take: Id },
    #[serde(rename_all = "camelCase")]
    FlattenComp { track: Id, group: Id },
    #[serde(rename_all = "camelCase")]
    InsertEffect { track: Option<Id>, effect: String, index: Option<usize> },
    #[serde(rename_all = "camelCase")]
    InsertPlugin { track: Option<Id>, plugin: PluginRef, index: Option<usize> },
    #[serde(rename_all = "camelCase")]
    RemoveEffect { track: Option<Id>, slot: Id },
    #[serde(rename_all = "camelCase")]
    MoveEffect { track: Option<Id>, slot: Id, index: usize },
    #[serde(rename_all = "camelCase")]
    SetEffectBypass { track: Option<Id>, slot: Id, bypass: bool },
    #[serde(rename_all = "camelCase")]
    SetEffectParam { track: Option<Id>, slot: Id, param: String, value: f32 },
    #[serde(rename_all = "camelCase")]
    SetEffectParams { track: Option<Id>, slot: Id, params: BTreeMap<String, f32> },
    #[serde(rename_all = "camelCase")]
    SetPluginState { track: Option<Id>, slot: Id, state: Option<String>, params: Option<BTreeMap<u32, f64>> },
    #[serde(rename_all = "camelCase")]
    RelinkPlugin { track: Option<Id>, slot: Id, plugin: PluginRef },
    #[serde(rename_all = "camelCase")]
    SetMasterGain { gain_db: f32 },
    #[serde(rename_all = "camelCase")]
    SetLoop { start: Option<u64>, end: Option<u64> },
    #[serde(rename_all = "camelCase")]
    SetTempo { bpm: f32 },
    #[serde(rename_all = "camelCase")]
    SetMetronome { metronome: Metronome },
    #[serde(rename_all = "camelCase")]
    RenameProject { name: String },
    #[serde(rename_all = "camelCase")]
    SetNotes { notes: String },
    #[serde(rename_all = "camelCase")]
    AddMarker { position: u64, label: String },
    #[serde(rename_all = "camelCase")]
    RemoveMarker { id: Id },
}

impl EditOp {
    fn label(&self) -> &'static str {
        match self {
            EditOp::AddTrack { .. } => "Add track",
            EditOp::RemoveTrack { .. } => "Delete track",
            EditOp::MoveTrack { .. } => "Reorder track",
            EditOp::UpdateTrack { .. } => "Track change",
            EditOp::MoveClip { .. } => "Move clip",
            EditOp::TrimClip { .. } => "Trim clip",
            EditOp::SplitClip { .. } => "Split clip",
            EditOp::DuplicateClip { .. } => "Duplicate clip",
            EditOp::DeleteClip { .. } => "Delete clip",
            EditOp::UpdateClip { .. } => "Clip change",
            EditOp::SetComp { .. } => "Choose take",
            EditOp::FlattenComp { .. } => "Flatten comp",
            EditOp::InsertEffect { .. } | EditOp::InsertPlugin { .. } => "Insert effect",
            EditOp::RemoveEffect { .. } => "Remove effect",
            EditOp::MoveEffect { .. } => "Reorder effect",
            EditOp::SetEffectBypass { .. } => "Bypass effect",
            EditOp::SetEffectParam { .. } | EditOp::SetEffectParams { .. } => "Effect parameter",
            EditOp::SetPluginState { .. } => "Plugin state",
            EditOp::RelinkPlugin { .. } => "Relink plugin",
            EditOp::SetMasterGain { .. } => "Master level",
            EditOp::SetLoop { .. } => "Loop region",
            EditOp::SetTempo { .. } => "Tempo",
            EditOp::SetMetronome { .. } => "Metronome",
            EditOp::RenameProject { .. } => "Rename project",
            EditOp::SetNotes { .. } => "Session notes",
            EditOp::AddMarker { .. } => "Add marker",
            EditOp::RemoveMarker { .. } => "Remove marker",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioState {
    pub project: Project,
    pub path: String,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub asset_checks: Vec<AssetCheck>,
    pub warnings: Vec<String>,
    pub recording: bool,
    pub newer_autosave: Option<RevisionInfo>,
    pub missing_plugins: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordStopResult {
    pub clips: Vec<Id>,
    pub latency_frames: u64,
    pub dropped_frames: u64,
    pub warnings: Vec<String>,
}

impl AppCore {
    fn state_of(s: &StudioSession) -> StudioState {
        StudioState {
            project: s.project.clone(),
            path: s.store.dir().to_string_lossy().to_string(),
            dirty: s.dirty,
            can_undo: s.history.can_undo(),
            can_redo: s.history.can_redo(),
            undo_label: s.history.undo_label().map(String::from),
            redo_label: s.history.redo_label().map(String::from),
            asset_checks: s.asset_checks.iter().filter(|c| c.status != AssetStatus::Ok).cloned().collect(),
            warnings: s.warnings.clone(),
            recording: s.recording.is_some(),
            newer_autosave: s.newer_autosave.clone(),
            missing_plugins: s.missing_plugins.clone(),
        }
    }

    pub fn studio_state(&self) -> Result<Option<StudioState>> {
        Ok(self.studio.lock().as_ref().map(Self::state_of))
    }

    fn open_session(&self, store: ProjectStore, loaded: LoadedProject) -> Result<StudioState> {
        let checks = store.validate_assets(&loaded.project);
        let mut warnings = Vec::new();
        if loaded.skipped_corrupt > 0 {
            warnings.push(format!(
                "{} damaged save{} skipped; the newest readable version was opened.",
                loaded.skipped_corrupt,
                if loaded.skipped_corrupt == 1 { " was" } else { "s were" }
            ));
        }
        if let Some(v) = loaded.migrated_from {
            warnings.push(format!("This project was upgraded from format version {v}. Saving will store it in the new format."));
        }
        let missing = checks.iter().filter(|c| c.status != AssetStatus::Ok).count();
        if missing > 0 {
            warnings.push(format!("{missing} audio file{} missing or unreadable. Those clips are silent until relinked.", if missing == 1 { " is" } else { "s are" }));
        }
        let p = &loaded.project;
        self.library.lock().touch_project(
            &p.id.to_string(),
            &p.name,
            &store.dir().to_string_lossy(),
            p.created_at,
            p.modified_at,
        )?;
        let mut session = StudioSession {
            store,
            project: loaded.project,
            history: History::default(),
            dirty: false,
            structure: vec![],
            transient_base: None,
            last_autosave: Instant::now(),
            asset_checks: checks,
            recording: None,
            warnings,
            newer_autosave: loaded.newer_autosave,
            missing_plugins: Vec::new(),
        };
        let mut studio = self.studio.lock();
        if let Some(old) = studio.as_ref() {
            if old.recording.is_some() {
                return Err(CoreError::Busy("stop recording before opening another project".into()));
            }
        }
        {
            let mut audio = self.audio.lock();
            if let Some(e) = audio.engine.as_mut() {
                let _ = e.stop();
                session.sync_engine(e, true);
            }
        }
        let state = Self::state_of(&session);
        *studio = Some(session);
        Ok(state)
    }

    /// Create a new project folder (in the default projects folder unless
    /// `folder` is given) with one vocal track, and open it.
    pub fn studio_new(&self, name: &str, folder: Option<PathBuf>, sample_rate: Option<u32>) -> Result<StudioState> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::Invalid("project name cannot be empty".into()));
        }
        self.studio_close_check()?;
        let base = folder.unwrap_or_else(|| self.paths.projects_dir.clone());
        std::fs::create_dir_all(&base)?;
        let stem = sanitize_file_stem(name);
        let mut dir = base.join(format!("{stem}.{PROJECT_EXT}"));
        let mut n = 2;
        while dir.exists() {
            dir = base.join(format!("{stem} {n}.{PROJECT_EXT}"));
            n += 1;
        }
        let rate = sample_rate.or_else(|| self.audio.lock().engine.as_ref().map(|e| e.sample_rate())).unwrap_or(48_000);
        if !uta_audio::device::COMMON_RATES.contains(&rate) {
            return Err(CoreError::Invalid(format!("unsupported project sample rate {rate}")));
        }
        let mut p = Project::new(name, rate);
        let t = edit::add_track(&mut p, Some("Vocal".into()));
        edit::update_track(&mut p, t, &TrackUpdate { armed: Some(true), ..Default::default() })?;
        let store = ProjectStore::create(&dir, &p)?;
        let (store, loaded) = {
            drop(store);
            ProjectStore::open(&dir)?
        };
        self.open_session(store, loaded)
    }

    pub fn studio_open(&self, dir: &Path) -> Result<StudioState> {
        self.studio_close_check()?;
        let (store, loaded) = ProjectStore::open(dir)?;
        self.open_session(store, loaded)
    }

    fn studio_close_check(&self) -> Result<()> {
        if let Some(s) = self.studio.lock().as_ref() {
            if s.recording.is_some() {
                return Err(CoreError::Busy("stop recording first".into()));
            }
        }
        Ok(())
    }

    /// Close the project. Unsaved changes are kept as an autosave revision
    /// (never silently discarded) unless `save` is true (manual save).
    pub fn studio_close(&self, save: bool) -> Result<()> {
        let mut studio = self.studio.lock();
        if let Some(s) = studio.as_mut() {
            if s.recording.is_some() {
                return Err(CoreError::Busy("stop recording first".into()));
            }
            if s.dirty {
                let kind = if save { RevisionKind::Save } else { RevisionKind::Autosave };
                s.project.modified_at = now();
                s.store.save(&s.project, kind)?;
            }
        }
        *studio = None;
        let mut audio = self.audio.lock();
        if let Some(e) = audio.engine.as_mut() {
            let _ = e.stop();
            let _ = e.set_graph(uta_audio::graph::Graph::new(e.sample_rate() as f32));
            e.set_sources(Default::default());
            let _ = e.set_loop(None);
        }
        Ok(())
    }

    pub fn studio_save(&self) -> Result<StudioState> {
        let mut studio = self.studio.lock();
        let s = studio.as_mut().ok_or(CoreError::NoProject)?;
        s.project.modified_at = now();
        s.store.save(&s.project, RevisionKind::Save)?;
        s.dirty = false;
        s.newer_autosave = None;
        let p = &s.project;
        self.library
            .lock()
            .touch_project(&p.id.to_string(), &p.name, &s.store.dir().to_string_lossy(), p.created_at, p.modified_at)?;
        Ok(Self::state_of(s))
    }

    pub fn studio_autosave_if_dirty(&self) -> Result<bool> {
        let minutes = self.settings().autosave_minutes as u64;
        let mut studio = self.studio.lock();
        let Some(s) = studio.as_mut() else { return Ok(false) };
        if !s.dirty || s.recording.is_some() || s.last_autosave.elapsed().as_secs() < minutes * 60 {
            return Ok(false);
        }
        s.store.save(&s.project, RevisionKind::Autosave)?;
        s.last_autosave = Instant::now();
        Ok(true)
    }

    /// Force an autosave now (used on shutdown and in tests).
    pub fn studio_autosave_now(&self) -> Result<()> {
        let mut studio = self.studio.lock();
        let s = studio.as_mut().ok_or(CoreError::NoProject)?;
        s.store.save(&s.project, RevisionKind::Autosave)?;
        s.last_autosave = Instant::now();
        Ok(())
    }

    pub fn studio_revisions(&self) -> Result<Vec<RevisionInfo>> {
        let studio = self.studio.lock();
        let s = studio.as_ref().ok_or(CoreError::NoProject)?;
        Ok(s.store.revisions()?)
    }

    /// Load an earlier revision (e.g. recover an autosave). The current state
    /// stays undoable.
    pub fn studio_load_revision(&self, id: i64) -> Result<StudioState> {
        let mut studio = self.studio.lock();
        let s = studio.as_mut().ok_or(CoreError::NoProject)?;
        let (p, _) = s.store.load_revision(id)?;
        s.history.record("Load saved version", s.project.clone());
        s.project = p;
        s.dirty = true;
        s.newer_autosave = None;
        s.asset_checks = s.store.validate_assets(&s.project);
        let mut audio = self.audio.lock();
        if let Some(e) = audio.engine.as_mut() {
            s.sync_engine(e, true);
        }
        Ok(Self::state_of(s))
    }

    pub fn studio_backup(&self) -> Result<PathBuf> {
        let studio = self.studio.lock();
        let s = studio.as_ref().ok_or(CoreError::NoProject)?;
        let dest = self.paths.backups_dir.join(format!(
            "{}-{}.{PROJECT_EXT}",
            sanitize_file_stem(&s.project.name),
            now().format("%Y%m%d-%H%M%S")
        ));
        s.store.backup_to(&dest)?;
        Ok(dest)
    }

    pub fn recent_projects(&self, query: Option<&str>) -> Result<Vec<uta_project::library::ProjectEntry>> {
        Ok(self.library.lock().recent_projects(30, query)?)
    }

    pub fn forget_project(&self, id: &str) -> Result<()> {
        Ok(self.library.lock().forget_project(id)?)
    }

    /// Apply an edit. `transient` edits (fader/knob drags) update the
    /// document and the audio immediately but create a single undo step when
    /// the drag is committed with a final non-transient edit.
    pub fn studio_edit(&self, op: EditOp, transient: bool) -> Result<Option<StudioState>> {
        let mut studio = self.studio.lock();
        let s = studio.as_mut().ok_or(CoreError::NoProject)?;
        if s.recording.is_some() && !matches!(op, EditOp::UpdateTrack { .. } | EditOp::SetEffectParam { .. } | EditOp::SetMasterGain { .. } | EditOp::SetNotes { .. }) {
            return Err(CoreError::Busy("this edit isn't possible while recording".into()));
        }
        let label = op.label();
        let before = s.project.clone();
        let mut p = s.project.clone();
        let mut rt: Vec<uta_audio::core::Command> = Vec::new();
        let mut sources_changed = false;
        match &op {
            EditOp::AddTrack { name } => {
                if p.tracks.len() >= MAX_TRACKS {
                    return Err(CoreError::Invalid(format!("a project can have at most {MAX_TRACKS} tracks")));
                }
                edit::add_track(&mut p, name.clone());
            }
            EditOp::RemoveTrack { track } => {
                edit::remove_track(&mut p, *track)?;
            }
            EditOp::MoveTrack { track, index } => edit::move_track(&mut p, *track, *index)?,
            EditOp::UpdateTrack { track, update } => {
                edit::update_track(&mut p, *track, update)?;
                rt.extend(mix_commands(&p));
            }
            EditOp::MoveClip { clip, position, track } => {
                edit::move_clip(&mut p, *clip, *position, *track)?;
                sources_changed = true;
            }
            EditOp::TrimClip { clip, start, end } => {
                edit::trim_clip(&mut p, *clip, *start, *end)?;
                sources_changed = true;
            }
            EditOp::SplitClip { clip, at } => {
                edit::split_clip(&mut p, *clip, *at)?;
                sources_changed = true;
            }
            EditOp::DuplicateClip { clip, position } => {
                edit::duplicate_clip(&mut p, *clip, *position)?;
                sources_changed = true;
            }
            EditOp::DeleteClip { clip } => {
                edit::delete_clip(&mut p, *clip)?;
                sources_changed = true;
            }
            EditOp::UpdateClip { clip, update } => {
                edit::update_clip(&mut p, *clip, update)?;
                sources_changed = true;
            }
            EditOp::SetComp { track, group, start, end, take } => {
                edit::set_comp(&mut p, *track, *group, *start, *end, *take)?;
                sources_changed = true;
            }
            EditOp::FlattenComp { track, group } => {
                edit::flatten_comp(&mut p, *track, *group)?;
                sources_changed = true;
            }
            EditOp::InsertEffect { track, effect, index } => {
                let kind = uta_dsp::BuiltinKind::from_id(effect).ok_or_else(|| CoreError::Invalid(format!("unknown effect {effect}")))?;
                edit::insert_effect(&mut p, *track, EffectSlot::builtin(kind), *index)?;
            }
            EditOp::InsertPlugin { track, plugin, index } => {
                let slot = EffectSlot {
                    id: new_id(),
                    bypass: false,
                    effect: EffectKind::Plugin { plugin: plugin.clone(), state: None, params: BTreeMap::new() },
                };
                edit::insert_effect(&mut p, *track, slot, *index)?;
            }
            EditOp::RemoveEffect { track, slot } => {
                edit::remove_effect(&mut p, *track, *slot)?;
            }
            EditOp::MoveEffect { track, slot, index } => edit::move_effect(&mut p, *track, *slot, *index)?,
            EditOp::SetEffectBypass { track, slot, bypass } => {
                edit::set_effect_bypass(&mut p, *track, *slot, *bypass)?;
                rt.push(uta_audio::core::Command::SetEffectBypass { track: track.map(|t| t.as_u128()), slot: slot.as_u128(), bypass: *bypass });
            }
            EditOp::SetEffectParam { track, slot, param, value } => {
                let v = edit::set_builtin_param(&mut p, *track, *slot, param, *value)?;
                if let Some(idx) = builtin_param_index(&p, *track, *slot, param) {
                    rt.push(uta_audio::core::Command::SetEffectParam { track: track.map(|t| t.as_u128()), slot: slot.as_u128(), param: idx, value: v });
                }
            }
            EditOp::SetEffectParams { track, slot, params } => {
                for (k, v) in params {
                    let v = edit::set_builtin_param(&mut p, *track, *slot, k, *v)?;
                    if let Some(idx) = builtin_param_index(&p, *track, *slot, k) {
                        rt.push(uta_audio::core::Command::SetEffectParam { track: track.map(|t| t.as_u128()), slot: slot.as_u128(), param: idx, value: v });
                    }
                }
            }
            EditOp::SetPluginState { track, slot, state, params } => {
                edit::set_plugin_state(&mut p, *track, *slot, state.clone(), params.clone())?;
            }
            EditOp::RelinkPlugin { track, slot, plugin } => {
                edit::relink_plugin(&mut p, *track, *slot, plugin.clone())?;
            }
            EditOp::SetMasterGain { gain_db } => {
                edit::set_master_gain(&mut p, *gain_db)?;
                rt.push(uta_audio::core::Command::SetMasterGain(uta_dsp::db_to_gain(p.master.gain_db)));
            }
            EditOp::SetLoop { start, end } => {
                let region = match (start, end) {
                    (Some(a), Some(b)) => Some(Region { start: *a.min(b), end: *a.max(b) }),
                    _ => None,
                };
                edit::set_loop(&mut p, region)?;
            }
            EditOp::SetTempo { bpm } => edit::set_tempo(&mut p, *bpm)?,
            EditOp::SetMetronome { metronome } => {
                let mut m = metronome.clone();
                m.volume_db = m.volume_db.clamp(-48.0, 6.0);
                m.count_in_bars = m.count_in_bars.min(2);
                p.metronome = m;
            }
            EditOp::RenameProject { name } => {
                let n = name.trim();
                if n.is_empty() {
                    return Err(CoreError::Invalid("project name cannot be empty".into()));
                }
                p.name = n.chars().take(80).collect();
            }
            EditOp::SetNotes { notes } => p.notes = notes.chars().take(20_000).collect(),
            EditOp::AddMarker { position, label } => p.markers.push(uta_project::Marker { id: new_id(), position: *position, label: label.chars().take(40).collect() }),
            EditOp::RemoveMarker { id } => p.markers.retain(|m| m.id != *id),
        }
        // history
        if transient {
            if s.transient_base.is_none() {
                s.transient_base = Some((label.to_string(), before));
            }
        } else {
            let base = s.transient_base.take().map(|(_, b)| b).unwrap_or(before);
            s.history.record(label, base);
        }
        s.project = p;
        s.dirty = true;
        // audio
        let mut audio = self.audio.lock();
        if let Some(e) = audio.engine.as_mut() {
            let key = structure_key(&s.project);
            if key != s.structure {
                s.sync_engine(e, false);
            } else {
                for c in rt {
                    let _ = e.send(c);
                }
                match &op {
                    EditOp::SetLoop { .. } => {
                        let _ = e.set_loop(s.project.loop_region.map(|r| (r.start, r.end)));
                    }
                    EditOp::SetTempo { .. } | EditOp::SetMetronome { .. } => {
                        let _ = e.set_metronome(uta_audio::core::MetronomeSettings {
                            enabled: s.project.metronome.enabled,
                            bpm: s.project.tempo_bpm,
                            beats_per_bar: s.project.time_signature.0 as u32,
                            gain: uta_dsp::db_to_gain(s.project.metronome.volume_db),
                        });
                    }
                    EditOp::SetPluginState { .. } | EditOp::RelinkPlugin { .. } => s.sync_engine(e, true),
                    _ => {}
                }
                if sources_changed {
                    e.set_sources(build_sources(&s.project, s.resolver()));
                    let _ = e.refresh_playback();
                }
            }
        }
        Ok(if transient { None } else { Some(Self::state_of(s)) })
    }

    pub fn studio_undo(&self) -> Result<StudioState> {
        self.history_step(true)
    }

    pub fn studio_redo(&self) -> Result<StudioState> {
        self.history_step(false)
    }

    fn history_step(&self, undo: bool) -> Result<StudioState> {
        let mut studio = self.studio.lock();
        let s = studio.as_mut().ok_or(CoreError::NoProject)?;
        if s.recording.is_some() {
            return Err(CoreError::Busy("stop recording first".into()));
        }
        s.transient_base = None;
        let done = if undo { s.history.undo(&mut s.project) } else { s.history.redo(&mut s.project) };
        if done.is_none() {
            return Err(CoreError::Invalid(if undo { "nothing to undo".into() } else { "nothing to redo".into() }));
        }
        s.dirty = true;
        let mut audio = self.audio.lock();
        if let Some(e) = audio.engine.as_mut() {
            s.sync_engine(e, true);
        }
        Ok(Self::state_of(s))
    }

    // ----------------------------------------------------------- transport

    pub fn studio_play(&self, from: Option<u64>) -> Result<()> {
        let studio = self.studio.lock();
        let s = studio.as_ref().ok_or(CoreError::NoProject)?;
        let mut audio = self.audio.lock();
        let e = audio.engine.as_mut().ok_or(CoreError::Audio(uta_audio::AudioError::NotRunning))?;
        s.sample_rate_ok(e)?;
        let pos = from.unwrap_or_else(|| e.position());
        Ok(e.play(pos)?)
    }

    pub fn studio_stop(&self) -> Result<()> {
        if self.studio.lock().as_ref().map(|s| s.recording.is_some()).unwrap_or(false) {
            self.studio_stop_recording()?;
            return Ok(());
        }
        self.with_engine(|e| Ok(e.stop()?))
    }

    pub fn studio_locate(&self, pos: u64) -> Result<()> {
        self.with_engine(|e| Ok(e.locate(pos)?))
    }

    /// Record onto every armed track from `from` (or the current position).
    pub fn studio_record(&self, from: Option<u64>) -> Result<()> {
        if !self.settings().microphone_enabled {
            return Err(CoreError::Invalid("Microphone access is off. Enable it in Settings to record.".into()));
        }
        if self.capture.lock().is_some() {
            return Err(CoreError::Busy("another recording is running".into()));
        }
        let mut studio = self.studio.lock();
        let s = studio.as_mut().ok_or(CoreError::NoProject)?;
        if s.recording.is_some() {
            return Err(CoreError::Busy("already recording".into()));
        }
        let armed: Vec<&uta_project::Track> = s.project.tracks.iter().filter(|t| t.armed).collect();
        if armed.is_empty() {
            return Err(CoreError::Invalid("Arm a track (R button) to record onto it.".into()));
        }
        let stamp = now().format("%Y%m%d-%H%M%S");
        let mut targets = Vec::new();
        let mut tracks = Vec::new();
        for t in &armed {
            let file = s.store.audio_dir().join(format!("{}-{stamp}-{}.wav", sanitize_file_stem(&t.name), &uuid::Uuid::new_v4().simple().to_string()[..6]));
            targets.push(RecordTarget { track_id: t.id.as_u128(), channel: t.input_channel, stereo: t.input_stereo, path: file.clone() });
            tracks.push((t.id, file));
        }
        let mut audio = self.audio.lock();
        let e = audio.engine.as_mut().ok_or(CoreError::Audio(uta_audio::AudioError::NotRunning))?;
        s.sample_rate_ok(e)?;
        let pos = from.unwrap_or_else(|| e.position());
        e.record(targets, pos)?;
        s.recording = Some(StudioRecording { tracks });
        Ok(())
    }

    /// Stop recording; new takes become clips (and take groups when they
    /// overlap earlier takes). One undo step.
    pub fn studio_stop_recording(&self) -> Result<RecordStopResult> {
        let mut studio = self.studio.lock();
        let s = studio.as_mut().ok_or(CoreError::NoProject)?;
        let rec = s.recording.take().ok_or_else(|| CoreError::Invalid("not recording".into()))?;
        let mut audio = self.audio.lock();
        let e = audio.engine.as_mut().ok_or(CoreError::Audio(uta_audio::AudioError::NotRunning))?;
        let out = e.stop_recording()?;
        let mut warnings = out.result.errors.clone();
        if out.result.dropped_frames > 0 || out.result.missing_frames > 0 {
            warnings.push(format!(
                "{} frames could not be recorded in time (the computer may be overloaded).",
                out.result.dropped_frames + out.result.missing_frames
            ));
        }
        let before = s.project.clone();
        let mut clips = Vec::new();
        for f in &out.result.files {
            let Some((track, _)) = rec.tracks.iter().find(|(t, _)| t.as_u128() == f.track_id) else { continue };
            if f.frames == 0 {
                let _ = std::fs::remove_file(&f.path);
                continue;
            }
            let rel = format!(
                "audio/{}",
                f.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
            );
            let asset = Asset {
                id: new_id(),
                file: rel,
                original_name: format!("{} take", s.project.track(*track).map(|t| t.name.as_str()).unwrap_or("Track")),
                kind: AssetKind::Recording,
                sample_rate: out.sample_rate,
                channels: f.channels,
                frames: f.frames,
                created_at: now(),
            };
            let aid = asset.id;
            s.project.assets.push(asset);
            // latency compensation: the take really started `latency` earlier
            let start = out.start_position;
            let lat = out.latency_frames;
            let cid = edit::add_clip(&mut s.project, *track, aid, start.saturating_sub(lat))?;
            if lat > start {
                // trim the part that would land before zero
                edit::trim_clip(&mut s.project, cid, Some(0), None)?;
                let skip = lat - start;
                let (ti, ci) = s.project.find_clip(cid).ok_or_else(|| CoreError::Other("clip vanished".into()))?;
                let c = &mut s.project.tracks[ti].clips[ci];
                c.source_offset = skip.min(f.frames.saturating_sub(1));
                c.length = f.frames - c.source_offset;
            }
            edit::register_take(&mut s.project, *track, cid)?;
            clips.push(cid);
        }
        if !clips.is_empty() {
            s.history.record("Record", before);
            s.dirty = true;
        }
        s.asset_checks = s.store.validate_assets(&s.project);
        e.set_sources(build_sources(&s.project, s.resolver()));
        Ok(RecordStopResult { clips, latency_frames: out.latency_frames, dropped_frames: out.result.dropped_frames, warnings })
    }

    // ------------------------------------------------------- import/export

    /// Import WAV files as new clips (background job). Files are converted to
    /// the project sample rate; originals are never modified.
    pub fn studio_import(self: &Arc<Self>, files: Vec<PathBuf>, track: Option<Id>, position: u64) -> Result<u64> {
        let (audio_dir, rate) = {
            let studio = self.studio.lock();
            let s = studio.as_ref().ok_or(CoreError::NoProject)?;
            (s.store.audio_dir(), s.project.sample_rate)
        };
        if files.is_empty() {
            return Err(CoreError::Invalid("no files selected".into()));
        }
        let core = self.clone();
        let label = format!("Importing {} file{}", files.len(), if files.len() == 1 { "" } else { "s" });
        Ok(self.jobs.submit("import", label, move |job| {
            let mut imported = Vec::new();
            for (i, src) in files.iter().enumerate() {
                if job.cancelled() {
                    return Err("cancelled".into());
                }
                let stem = src.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "import".into());
                let dest = audio_dir.join(format!("{}-{}.wav", sanitize_file_stem(&stem), &uuid::Uuid::new_v4().simple().to_string()[..6]));
                let info = uta_audio::import::import_wav(src, &dest, rate).map_err(|e| e.to_string())?;
                imported.push((stem, dest, info));
                job.set_progress((i + 1) as f32 / files.len() as f32);
            }
            let ids = core.add_imported(imported, track, position).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "clips": ids }))
        }))
    }

    fn add_imported(&self, items: Vec<(String, PathBuf, uta_audio::wav::WavInfo)>, track: Option<Id>, position: u64) -> Result<Vec<Id>> {
        let mut studio = self.studio.lock();
        let s = studio.as_mut().ok_or(CoreError::NoProject)?;
        let before = s.project.clone();
        let mut ids = Vec::new();
        let mut target = track;
        for (name, path, info) in items {
            let t = match target {
                Some(t) if s.project.track(t).is_some() => t,
                _ => {
                    if s.project.tracks.len() >= MAX_TRACKS {
                        return Err(CoreError::Invalid("track limit reached".into()));
                    }
                    edit::add_track(&mut s.project, Some(name.chars().take(30).collect()))
                }
            };
            target = None; // each further file gets its own track
            let asset = Asset {
                id: new_id(),
                file: format!("audio/{}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
                original_name: name,
                kind: AssetKind::Import,
                sample_rate: info.sample_rate,
                channels: info.channels,
                frames: info.frames,
                created_at: now(),
            };
            let aid = asset.id;
            s.project.assets.push(asset);
            ids.push(edit::add_clip(&mut s.project, t, aid, position)?);
        }
        s.history.record("Import audio", before);
        s.dirty = true;
        s.asset_checks = s.store.validate_assets(&s.project);
        let mut audio = self.audio.lock();
        if let Some(e) = audio.engine.as_mut() {
            s.sync_engine(e, false);
        }
        Ok(ids)
    }

    /// Replace a missing asset with another WAV file (copied/converted into
    /// the project). The asset id stays the same, so clips reconnect.
    pub fn studio_relink_asset(&self, asset: Id, file: &Path) -> Result<StudioState> {
        let mut studio = self.studio.lock();
        let s = studio.as_mut().ok_or(CoreError::NoProject)?;
        let idx = s.project.assets.iter().position(|a| a.id == asset).ok_or_else(|| CoreError::NotFound("asset".into()))?;
        let stem = file.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_else(|| "relinked".into());
        let dest = s.store.audio_dir().join(format!("{}-{}.wav", sanitize_file_stem(&stem), &uuid::Uuid::new_v4().simple().to_string()[..6]));
        let info = uta_audio::import::import_wav(file, &dest, s.project.sample_rate)?;
        let before = s.project.clone();
        let a = &mut s.project.assets[idx];
        a.file = format!("audio/{}", dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        a.sample_rate = info.sample_rate;
        a.channels = info.channels;
        let shorter = info.frames < a.frames;
        a.frames = info.frames;
        if shorter {
            // keep clips inside the new file
            let frames = info.frames;
            for t in &mut s.project.tracks {
                for c in &mut t.clips {
                    if c.asset_id == asset {
                        c.source_offset = c.source_offset.min(frames.saturating_sub(1));
                        c.length = c.length.min(frames - c.source_offset);
                    }
                }
            }
        }
        s.history.record("Relink audio", before);
        s.dirty = true;
        s.asset_checks = s.store.validate_assets(&s.project);
        s.warnings.retain(|w| !w.contains("missing or unreadable"));
        let mut audio = self.audio.lock();
        if let Some(e) = audio.engine.as_mut() {
            e.set_sources(build_sources(&s.project, s.resolver()));
        }
        Ok(Self::state_of(s))
    }

    /// Render the mix to a WAV file (background job).
    pub fn studio_export(self: &Arc<Self>, path: PathBuf, mut opts: RenderOptions) -> Result<u64> {
        let (project, sources, rate) = {
            let studio = self.studio.lock();
            let s = studio.as_ref().ok_or(CoreError::NoProject)?;
            let resolver = s.resolver();
            (s.project.clone(), build_sources(&s.project, resolver), s.project.sample_rate)
        };
        if opts.end == 0 {
            opts.end = project.length_frames();
        }
        if opts.end <= opts.start {
            return Err(CoreError::Invalid("there is nothing to export in that range".into()));
        }
        if path.extension().map(|e| e.to_ascii_lowercase() != "wav").unwrap_or(true) {
            return Err(CoreError::Invalid("exports are WAV files (.wav)".into()));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let missing = sources.missing_assets.len();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        Ok(self.jobs.submit("export", format!("Exporting {name}"), move |job| {
            let graph = build_graph(&project, rate as f32, &mut NoPlugins);
            let mut report = render_to_wav(graph, Arc::new(sources), rate, &opts, &path, |p| job.report(p)).map_err(|e| e.to_string())?;
            if missing > 0 {
                report.warnings.push(format!("{missing} missing audio file(s) were rendered as silence."));
            }
            serde_json::to_value(report).map_err(|e| e.to_string())
        }))
    }

    pub fn studio_asset_waveform(&self, asset: Id, start: u64, end: Option<u64>, buckets: usize) -> Result<Vec<f32>> {
        let (path, cache) = {
            let studio = self.studio.lock();
            let s = studio.as_ref().ok_or(CoreError::NoProject)?;
            let a = s.project.asset(asset).ok_or_else(|| CoreError::NotFound("asset".into()))?;
            (s.store.asset_path(a)?, s.store.cache_dir())
        };
        if !path.is_file() {
            return Err(CoreError::NotFound("audio file".into()));
        }
        let peaks = uta_audio::waveform::Peaks::cached(&path, &cache)?;
        Ok(peaks.range(start, end.unwrap_or(peaks.frames), buckets))
    }
}

fn builtin_param_index(p: &Project, track: Option<Id>, slot: Id, param: &str) -> Option<u32> {
    let chain = match track {
        Some(t) => &p.track(t)?.effects,
        None => &p.master.effects,
    };
    let s = chain.iter().find(|s| s.id == slot)?;
    match &s.effect {
        EffectKind::Builtin { kind, .. } => kind.params().iter().position(|i| i.id == param).map(|i| i as u32),
        EffectKind::Plugin { .. } => None,
    }
}
