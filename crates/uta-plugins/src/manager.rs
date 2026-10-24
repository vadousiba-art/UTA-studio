//! Live plugin instances, one helper process per effect slot.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use uta_audio::graph::{ChainProcessor, MAX_BLOCK};
use uta_audio::graph_build::PluginFactory;
use uta_plugin_protocol::{ParamDesc, ParamValue, Request, Response};
use uta_project::{EffectKind, EffectSlot, PluginRef, Project};

use crate::bridge::{Link, OfflineProcessor, RtEndpoint, BRIDGE_LATENCY};
use crate::client::{ClientError, HelperClient, LOAD_TIMEOUT, REQUEST_TIMEOUT};
use crate::state;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstanceStatus {
    /// Loaded and processing audio.
    Running,
    /// The helper crashed or hung; the slot passes the dry signal until the
    /// plugin is reloaded.
    Crashed,
    /// The plugin is not installed on this computer.
    Missing,
    /// The plugin is installed but could not be loaded.
    Failed,
    /// The user disabled this plugin in the Plugin Manager.
    Disabled,
    /// Plugin hosting is not available (helper not found).
    Unavailable,
}

/// What the UI shows for a plugin slot.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotStatus {
    pub slot: String,
    pub track: Option<String>,
    pub name: String,
    pub vendor: String,
    pub status: InstanceStatus,
    pub message: Option<String>,
    /// Total latency this slot adds (bridge + plugin), frames.
    pub latency_frames: usize,
    pub late_blocks: u64,
    pub warnings: Vec<String>,
}

struct LiveInstance {
    plugin: PluginRef,
    path: PathBuf,
    link: Arc<Link>,
    params: Vec<ParamDesc>,
    warnings: Vec<String>,
    sample_rate: f32,
    applied_state: Option<String>,
    applied_params: BTreeMap<u32, f64>,
}

/// Owns live plugin instances and turns project slots into graph processors.
pub struct PluginManager {
    helper: Option<PathBuf>,
    instances: HashMap<uuid::Uuid, LiveInstance>,
    failures: HashMap<uuid::Uuid, (InstanceStatus, String)>,
    /// Installed plugins by class id → bundle path (from the scan cache), used
    /// when a project was made on another computer.
    known: HashMap<String, PathBuf>,
    disabled: HashSet<String>,
}

fn plugin_parts(slot: &EffectSlot) -> Option<(&PluginRef, &Option<String>, &BTreeMap<u32, f64>)> {
    match &slot.effect {
        EffectKind::Plugin { plugin, state, params } => Some((plugin, state, params)),
        EffectKind::Builtin { .. } => None,
    }
}

/// Spawn a helper and load a plugin into it.
fn load_into_helper(
    helper: &Path,
    path: &Path,
    plugin: &PluginRef,
    stored_state: Option<&str>,
    params: &BTreeMap<u32, f64>,
    sample_rate: f32,
    offline: bool,
) -> Result<(HelperClient, Vec<ParamDesc>, usize, Vec<String>), String> {
    let mut client = HelperClient::spawn(helper).map_err(|e| e.to_string())?;
    let (cs, ctl) = match stored_state.map(state::decode) {
        Some(Some((c, k))) => (Some(c), k),
        Some(None) => (None, None),
        None => (None, None),
    };
    let mut warnings = Vec::new();
    if stored_state.is_some() && cs.is_none() {
        warnings.push("the saved plugin state was in an unknown format and was ignored".into());
    }
    let req = Request::Load {
        path: path.to_string_lossy().to_string(),
        uid: plugin.uid.clone(),
        sample_rate: sample_rate as f64,
        max_block: MAX_BLOCK as u32,
        offline,
        component_state: cs,
        controller_state: ctl,
    };
    match client.request(&req, LOAD_TIMEOUT) {
        Ok(Response::Loaded { params: descs, latency_samples, warnings: w, .. }) => {
            warnings.extend(w);
            if !params.is_empty() {
                let values: Vec<(u32, f64)> = params.iter().map(|(k, v)| (*k, *v)).collect();
                if let Err(e) = client.request(&Request::SetParams { values }, REQUEST_TIMEOUT) {
                    warnings.push(format!("saved parameter values could not be applied: {e}"));
                }
            }
            Ok((client, descs, latency_samples as usize, warnings))
        }
        Ok(r) => Err(format!("unexpected reply {r:?}")),
        Err(e) => Err(e.to_string()),
    }
}

impl PluginManager {
    pub fn new(helper: Option<PathBuf>) -> Self {
        Self { helper, instances: HashMap::new(), failures: HashMap::new(), known: HashMap::new(), disabled: HashSet::new() }
    }

    pub fn helper(&self) -> Option<&Path> {
        self.helper.as_deref()
    }

    /// Update the installed-plugin index (uid → bundle path) and the set of
    /// disabled bundle paths from the scan cache.
    pub fn set_catalog(&mut self, known: HashMap<String, PathBuf>, disabled: HashSet<String>) {
        self.known = known;
        self.disabled = disabled;
    }

    /// Where to load `plugin` from on this computer.
    pub fn resolve(&self, plugin: &PluginRef) -> Option<PathBuf> {
        let p = PathBuf::from(&plugin.path);
        if !plugin.path.is_empty() && p.exists() {
            return Some(p);
        }
        self.known.get(&plugin.uid).cloned()
    }

    fn is_disabled(&self, path: &Path) -> bool {
        self.disabled.contains(&path.to_string_lossy().to_string())
    }

    /// Bring an existing instance in line with the slot (after undo or a
    /// revision load the stored state/params may differ).
    fn sync_instance(inst: &mut LiveInstance, stored: &Option<String>, params: &BTreeMap<u32, f64>) {
        if stored.is_some() && *stored != inst.applied_state {
            if let Some(Some((c, k))) = stored.as_deref().map(state::decode) {
                let r = inst.link.with_client(|cl| cl.request(&Request::SetState { component_state: c, controller_state: k }, REQUEST_TIMEOUT));
                if let Err(e) = r {
                    inst.warnings.push(format!("could not restore the plugin state: {e}"));
                }
            }
            inst.applied_state = stored.clone();
        }
        let changed: Vec<(u32, f64)> = params
            .iter()
            .filter(|(k, v)| inst.applied_params.get(k) != Some(v))
            .map(|(k, v)| (*k, *v))
            .collect();
        if !changed.is_empty() {
            let _ = inst.link.with_client(|cl| cl.request(&Request::SetParams { values: changed }, REQUEST_TIMEOUT));
            inst.applied_params = params.clone();
        }
    }

    fn create_rt(&mut self, slot: &EffectSlot, sample_rate: f32) -> Option<Box<dyn ChainProcessor>> {
        let (plugin, stored, params) = plugin_parts(slot)?;
        if let Some(inst) = self.instances.get_mut(&slot.id) {
            let same = inst.plugin.uid == plugin.uid && inst.sample_rate == sample_rate;
            if same && !inst.link.is_failed() {
                Self::sync_instance(inst, stored, params);
                return Some(Box::new(RtEndpoint::new(inst.link.clone())));
            }
            if same && inst.link.is_failed() {
                // stays crashed until the user reloads it
                return None;
            }
            let old = self.instances.remove(&slot.id);
            drop(old);
        }
        if let Some((InstanceStatus::Failed | InstanceStatus::Crashed, _)) = self.failures.get(&slot.id) {
            // don't retry a failing plugin on every graph rebuild
            return None;
        }
        self.failures.remove(&slot.id);
        let Some(helper) = self.helper.clone() else {
            self.failures.insert(slot.id, (InstanceStatus::Unavailable, "the plugin helper (uta-plugin-host) was not found next to the application".into()));
            return None;
        };
        let Some(path) = self.resolve(plugin) else {
            self.failures.insert(
                slot.id,
                (
                    InstanceStatus::Missing,
                    format!("{} is not installed on this computer (or has not been scanned). The slot passes audio through unchanged and keeps its saved settings.", plugin.name),
                ),
            );
            return None;
        };
        if self.is_disabled(&path) {
            self.failures.insert(slot.id, (InstanceStatus::Disabled, format!("{} is disabled in the Plugin Manager", plugin.name)));
            return None;
        }
        match load_into_helper(&helper, &path, plugin, stored.as_deref(), params, sample_rate, false) {
            Ok((client, descs, latency, warnings)) => {
                let link = Link::new(client, latency);
                let ep = RtEndpoint::new(link.clone());
                self.instances.insert(
                    slot.id,
                    LiveInstance {
                        plugin: plugin.clone(),
                        path,
                        link,
                        params: descs,
                        warnings,
                        sample_rate,
                        applied_state: stored.clone(),
                        applied_params: params.clone(),
                    },
                );
                Some(Box::new(ep))
            }
            Err(e) => {
                self.failures.insert(slot.id, (InstanceStatus::Failed, e));
                None
            }
        }
    }

    /// Shut down instances whose slots are no longer in the project.
    pub fn retain_project(&mut self, project: &Project) {
        let ids: HashSet<uuid::Uuid> = plugin_slots(project).map(|(_, s)| s.id).collect();
        self.instances.retain(|k, _| ids.contains(k));
        self.failures.retain(|k, _| ids.contains(k));
    }

    /// Forget an instance (and its failure) so the next graph build loads
    /// the plugin afresh. Used by "Reload plugin" and after relinking.
    pub fn reset_slot(&mut self, slot: uuid::Uuid) {
        self.instances.remove(&slot);
        self.failures.remove(&slot);
    }

    /// Unload every instance loaded from `path` (bundle disabled).
    pub fn unload_path(&mut self, path: &Path) {
        self.instances.retain(|_, i| i.path != path);
    }

    /// Clear every recorded failure (after a rescan or enabling plugins).
    pub fn clear_failures(&mut self) {
        self.failures.clear();
    }

    pub fn shutdown_all(&mut self) {
        self.instances.clear();
        self.failures.clear();
    }

    /// Current state of every running instance, encoded for the project.
    /// Crashed instances are skipped (their last saved state is kept).
    pub fn capture_states(&mut self) -> HashMap<uuid::Uuid, String> {
        let mut out = HashMap::new();
        for (id, inst) in self.instances.iter_mut() {
            if inst.link.is_failed() {
                continue;
            }
            match inst.link.with_client(|c| c.request(&Request::GetState, REQUEST_TIMEOUT)) {
                Ok(Response::State { component_state, controller_state }) => {
                    let s = state::encode(&component_state, controller_state.as_deref());
                    inst.applied_state = Some(s.clone());
                    out.insert(*id, s);
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("could not read state of {}: {e}", inst.plugin.name),
            }
        }
        out
    }

    /// Write captured states into the project (no-op for unchanged slots).
    /// Returns true when anything changed.
    pub fn store_states(&mut self, project: &mut Project) -> bool {
        let states = self.capture_states();
        let mut changed = false;
        let mut apply = |slots: &mut Vec<EffectSlot>| {
            for s in slots.iter_mut() {
                if let (Some(new), EffectKind::Plugin { state, .. }) = (states.get(&s.id), &mut s.effect) {
                    if state.as_ref() != Some(new) {
                        *state = Some(new.clone());
                        changed = true;
                    }
                }
            }
        };
        for t in project.tracks.iter_mut() {
            apply(&mut t.effects);
        }
        apply(&mut project.master.effects);
        changed
    }

    fn instance(&self, slot: uuid::Uuid) -> Result<&LiveInstance, ClientError> {
        match self.instances.get(&slot) {
            Some(i) if !i.link.is_failed() => Ok(i),
            Some(i) => Err(ClientError::Plugin(format!("{} has stopped working; reload it to continue", i.plugin.name))),
            None => Err(ClientError::Plugin("this plugin is not loaded".into())),
        }
    }

    pub fn param_descs(&self, slot: uuid::Uuid) -> Result<Vec<ParamDesc>, ClientError> {
        Ok(self.instance(slot)?.params.clone())
    }

    pub fn param_values(&self, slot: uuid::Uuid) -> Result<Vec<ParamValue>, ClientError> {
        let inst = self.instance(slot)?;
        match inst.link.with_client(|c| c.request(&Request::GetParams, REQUEST_TIMEOUT))? {
            Response::Params { values } => Ok(values),
            r => Err(ClientError::Protocol(format!("unexpected reply {r:?}"))),
        }
    }

    pub fn set_params(&mut self, slot: uuid::Uuid, values: &[(u32, f64)]) -> Result<(), ClientError> {
        let inst = self.instance(slot)?;
        inst.link
            .with_client(|c| c.request(&Request::SetParams { values: values.to_vec() }, REQUEST_TIMEOUT))?;
        if let Some(i) = self.instances.get_mut(&slot) {
            for (k, v) in values {
                i.applied_params.insert(*k, v.clamp(0.0, 1.0));
            }
        }
        Ok(())
    }

    /// Test hook: make the helper of `slot` abort.
    pub fn crash_for_testing(&self, slot: uuid::Uuid) {
        if let Some(i) = self.instances.get(&slot) {
            let _ = i.link.with_client(|c| c.request(&Request::CrashForTesting, std::time::Duration::from_secs(5)));
        }
    }

    pub fn status(&self, track: Option<uuid::Uuid>, slot: &EffectSlot) -> Option<SlotStatus> {
        let (plugin, _, _) = plugin_parts(slot)?;
        let mut st = SlotStatus {
            slot: slot.id.to_string(),
            track: track.map(|t| t.to_string()),
            name: plugin.name.clone(),
            vendor: plugin.vendor.clone(),
            status: InstanceStatus::Failed,
            message: None,
            latency_frames: 0,
            late_blocks: 0,
            warnings: Vec::new(),
        };
        if let Some(inst) = self.instances.get(&slot.id) {
            st.latency_frames = BRIDGE_LATENCY + inst.link.plugin_latency;
            st.late_blocks = inst.link.late_blocks.load(std::sync::atomic::Ordering::Relaxed);
            st.warnings = inst.warnings.clone();
            if inst.link.is_failed() {
                st.status = InstanceStatus::Crashed;
                st.message = Some(format!(
                    "{} stopped working ({}). The slot passes the unprocessed signal; use Reload to start it again.",
                    plugin.name,
                    inst.link.failure().unwrap_or_else(|| "unknown error".into())
                ));
            } else {
                st.status = InstanceStatus::Running;
                st.message = Some(format!("hosted in helper process {} from {}", inst.link.client.lock().pid, inst.path.display()));
            }
        } else if let Some((s, m)) = self.failures.get(&slot.id) {
            st.status = *s;
            st.message = Some(m.clone());
        } else {
            st.status = if self.helper.is_none() { InstanceStatus::Unavailable } else { InstanceStatus::Failed };
            st.message = Some("not loaded".into());
        }
        Some(st)
    }

    pub fn statuses(&self, project: &Project) -> Vec<SlotStatus> {
        plugin_slots(project).filter_map(|(t, s)| self.status(t, s)).collect()
    }

    /// Factory for building the live graph.
    pub fn factory(&mut self) -> LiveFactory<'_> {
        LiveFactory { manager: self }
    }

    /// Factory for an export: separate helpers in offline mode, loaded with
    /// the given project's stored states (call `store_states` first).
    pub fn offline_factory(&self) -> OfflineFactory {
        OfflineFactory {
            helper: self.helper.clone(),
            known: self.known.clone(),
            disabled: self.disabled.clone(),
            errors: Arc::new(Mutex::new(Vec::new())),
            helpers_started: 0,
        }
    }
}

/// All plugin slots of a project with their track (None = master).
pub fn plugin_slots(project: &Project) -> impl Iterator<Item = (Option<uuid::Uuid>, &EffectSlot)> {
    project
        .tracks
        .iter()
        .flat_map(|t| t.effects.iter().map(move |s| (Some(t.id), s)))
        .chain(project.master.effects.iter().map(|s| (None, s)))
        .filter(|(_, s)| matches!(s.effect, EffectKind::Plugin { .. }))
}

pub struct LiveFactory<'a> {
    manager: &'a mut PluginManager,
}

impl PluginFactory for LiveFactory<'_> {
    fn create(&mut self, _track: Option<uuid::Uuid>, slot: &EffectSlot, sample_rate: f32) -> Option<Box<dyn ChainProcessor>> {
        self.manager.create_rt(slot, sample_rate)
    }
}

pub struct OfflineFactory {
    helper: Option<PathBuf>,
    known: HashMap<String, PathBuf>,
    disabled: HashSet<String>,
    errors: Arc<Mutex<Vec<String>>>,
    pub helpers_started: usize,
}

impl OfflineFactory {
    /// Problems met while building or rendering (for the export report).
    pub fn errors(&self) -> Arc<Mutex<Vec<String>>> {
        self.errors.clone()
    }
}

impl PluginFactory for OfflineFactory {
    fn create(&mut self, _track: Option<uuid::Uuid>, slot: &EffectSlot, sample_rate: f32) -> Option<Box<dyn ChainProcessor>> {
        let (plugin, stored, params) = plugin_parts(slot)?;
        let Some(helper) = self.helper.clone() else {
            self.errors.lock().push(format!("{} was skipped: plugin hosting is unavailable", plugin.name));
            return None;
        };
        let p = PathBuf::from(&plugin.path);
        let path = if p.exists() { Some(p) } else { self.known.get(&plugin.uid).cloned() };
        let Some(path) = path else {
            self.errors.lock().push(format!("{} is not installed; its slot was exported unprocessed", plugin.name));
            return None;
        };
        if self.disabled.contains(&path.to_string_lossy().to_string()) {
            self.errors.lock().push(format!("{} is disabled; its slot was exported unprocessed", plugin.name));
            return None;
        }
        match load_into_helper(&helper, &path, plugin, stored.as_deref(), params, sample_rate, true) {
            Ok((client, _, latency, _)) => {
                self.helpers_started += 1;
                Some(Box::new(OfflineProcessor::new(client, plugin.name.clone(), latency, self.errors.clone())))
            }
            Err(e) => {
                self.errors.lock().push(format!("{} could not be loaded for export ({e}); its slot was exported unprocessed", plugin.name));
                None
            }
        }
    }
}
