//! The versioned project document.
//!
//! All timeline positions and lengths are in *frames at the project sample
//! rate*. Every audio asset referenced by a project is stored at the project
//! sample rate (imports and device-rate recordings are converted on ingest),
//! so the playback engine never resamples in real time.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Current project document format. Bump and add a migration in
/// [`crate::migrate`] whenever the serialized shape changes.
pub const FORMAT_VERSION: u32 = 2;

pub type Id = Uuid;

pub fn new_id() -> Id {
    Uuid::new_v4()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub format_version: u32,
    pub id: Id,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub modified_at: DateTime<Utc>,
    pub sample_rate: u32,
    pub tempo_bpm: f32,
    pub time_signature: (u8, u8),
    pub tracks: Vec<Track>,
    pub master: MasterBus,
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub loop_region: Option<Region>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default)]
    pub metronome: Metronome,
    /// Free-form session notes.
    #[serde(default)]
    pub notes: String,
    /// Training attempts (library ids) associated with this project.
    #[serde(default)]
    pub linked_attempts: Vec<Id>,
}

impl Project {
    pub fn new(name: impl Into<String>, sample_rate: u32) -> Self {
        let now = Utc::now();
        Self {
            format_version: FORMAT_VERSION,
            id: new_id(),
            name: name.into(),
            created_at: now,
            modified_at: now,
            sample_rate,
            tempo_bpm: 120.0,
            time_signature: (4, 4),
            tracks: Vec::new(),
            master: MasterBus::default(),
            assets: Vec::new(),
            loop_region: None,
            markers: Vec::new(),
            metronome: Metronome::default(),
            notes: String::new(),
            linked_attempts: Vec::new(),
        }
    }

    pub fn track(&self, id: Id) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: Id) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn asset(&self, id: Id) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }

    /// Find a clip anywhere in the project: `(track index, clip index)`.
    pub fn find_clip(&self, clip: Id) -> Option<(usize, usize)> {
        self.tracks.iter().enumerate().find_map(|(ti, t)| {
            t.clips.iter().position(|c| c.id == clip).map(|ci| (ti, ci))
        })
    }

    /// End of the last clip on the timeline, in frames.
    pub fn length_frames(&self) -> u64 {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.end())
            .max()
            .unwrap_or(0)
    }

    pub fn any_solo(&self) -> bool {
        self.tracks.iter().any(|t| t.solo)
    }

    /// Whether a track is audible given mute/solo state.
    pub fn is_track_audible(&self, track: &Track) -> bool {
        if track.mute {
            return false;
        }
        !self.any_solo() || track.solo
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub start: u64,
    pub end: u64,
}

impl Region {
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub id: Id,
    pub position: u64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metronome {
    pub enabled: bool,
    pub volume_db: f32,
    pub count_in_bars: u8,
}

impl Default for Metronome {
    fn default() -> Self {
        Self {
            enabled: false,
            volume_db: -12.0,
            count_in_bars: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MonitorMode {
    /// Never pass the live input to the output.
    Off,
    /// Monitor while the track is armed.
    Armed,
    /// Always monitor.
    On,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: Id,
    pub name: String,
    pub color: String,
    pub gain_db: f32,
    /// -1 (left) .. 1 (right).
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    pub monitor: MonitorMode,
    /// First input channel used when recording (0-based).
    pub input_channel: u16,
    /// Record two channels (`input_channel`, `input_channel + 1`).
    pub input_stereo: bool,
    pub clips: Vec<Clip>,
    pub effects: Vec<EffectSlot>,
    /// Take groups (recorded on top of each other) and their comp.
    #[serde(default)]
    pub take_groups: Vec<TakeGroup>,
    pub height: u16,
}

impl Track {
    pub fn new(name: impl Into<String>, color: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            color: color.into(),
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            armed: false,
            monitor: MonitorMode::Armed,
            input_channel: 0,
            input_stereo: false,
            clips: Vec::new(),
            effects: Vec::new(),
            take_groups: Vec::new(),
            height: 96,
        }
    }

    pub fn clip(&self, id: Id) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id == id)
    }

    pub fn take_group_of(&self, clip: Id) -> Option<&TakeGroup> {
        self.take_groups.iter().find(|g| g.takes.contains(&clip))
    }
}

/// A region of an asset placed on the timeline. Non-destructive: the asset
/// file is never modified by clip edits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: Id,
    pub asset_id: Id,
    pub name: String,
    /// Timeline position of the clip start (frames).
    pub position: u64,
    /// Offset into the asset where playback starts (frames).
    pub source_offset: u64,
    /// Length on the timeline (frames).
    pub length: u64,
    pub gain_db: f32,
    pub fade_in: u64,
    pub fade_out: u64,
    pub muted: bool,
}

impl Clip {
    pub fn end(&self) -> u64 {
        self.position + self.length
    }
}

/// Several takes recorded over the same time range. Only the comp segments
/// are played; each segment chooses which take is heard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TakeGroup {
    pub id: Id,
    /// Clip ids, oldest first.
    pub takes: Vec<Id>,
    pub comp: Vec<CompSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompSegment {
    pub start: u64,
    pub end: u64,
    pub take: Id,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetKind {
    Recording,
    Import,
    Render,
}

/// An audio file inside the project's `audio/` folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: Id,
    /// Path relative to the project directory, always under `audio/`.
    pub file: String,
    pub original_name: String,
    pub kind: AssetKind,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterBus {
    pub gain_db: f32,
    pub effects: Vec<EffectSlot>,
}

impl Default for MasterBus {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            effects: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectSlot {
    pub id: Id,
    pub bypass: bool,
    pub effect: EffectKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EffectKind {
    /// Built-in effect; `params` maps parameter id to value. Unknown ids are
    /// preserved but ignored, missing ids use defaults.
    #[serde(rename_all = "camelCase")]
    Builtin {
        kind: uta_dsp::BuiltinKind,
        params: BTreeMap<String, f32>,
    },
    /// Third-party plugin. The stored state is kept verbatim even when the
    /// plugin is missing on this computer, so nothing is lost.
    #[serde(rename_all = "camelCase")]
    Plugin {
        plugin: PluginRef,
        /// Opaque plugin state (component + controller) as written by the
        /// plugin host: `vst3:<component hex>[:<controller hex>]`.
        #[serde(default)]
        state: Option<String>,
        /// Normalised parameter values by VST3 parameter id, applied on top
        /// of the state (and used alone when the state cannot be restored).
        #[serde(default, with = "u32_keys")]
        params: BTreeMap<u32, f64>,
    },
}

/// JSON object keys are strings, and integer keys cannot be read back
/// through an internally tagged enum, so plugin parameter maps are stored
/// with explicit string keys.
mod u32_keys {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(m: &BTreeMap<u32, f64>, s: S) -> Result<S::Ok, S::Error> {
        m.iter().map(|(k, v)| (k.to_string(), *v)).collect::<BTreeMap<String, f64>>().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<u32, f64>, D::Error> {
        let raw = BTreeMap::<String, f64>::deserialize(d)?;
        raw.into_iter()
            .map(|(k, v)| k.parse::<u32>().map(|k| (k, v)).map_err(serde::de::Error::custom))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRef {
    /// Format, currently always `"vst3"`.
    pub format: String,
    /// Plugin class id (32 hex digits for VST3).
    pub uid: String,
    pub name: String,
    pub vendor: String,
    #[serde(default)]
    pub version: String,
    /// Path to the plugin bundle when it was inserted (used to relink).
    pub path: String,
}

impl EffectSlot {
    pub fn builtin(kind: uta_dsp::BuiltinKind) -> Self {
        let params = kind
            .params()
            .iter()
            .map(|p| (p.id.to_string(), p.default))
            .collect();
        Self {
            id: new_id(),
            bypass: false,
            effect: EffectKind::Builtin { kind, params },
        }
    }

    pub fn display_name(&self) -> String {
        match &self.effect {
            EffectKind::Builtin { kind, .. } => kind.display_name().to_string(),
            EffectKind::Plugin { plugin, .. } => plugin.name.clone(),
        }
    }
}

/// Track colour palette (cycled for new tracks).
pub const TRACK_COLORS: &[&str] = &["#3b9eff", "#7c5cff", "#2fd3b6", "#ff6b8b", "#f5b041", "#5dd16f", "#ff8a3d", "#a3b8cc"];
