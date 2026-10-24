//! Plugin Manager: scanning installed VST3 effects, enabling/disabling
//! them, and controlling plugin slots in the open project.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use uta_plugins::{default_folders, discover, scan_bundle, ParamDesc, ParamValue, PluginInfo, ScanOutcome};
use uta_project::library::PluginCacheEntry;
use uta_project::{EffectKind, Id};

use crate::error::{CoreError, Result};
use crate::AppCore;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginBundle {
    pub path: String,
    pub enabled: bool,
    /// Why the bundle could not be used (scan failure, crash, timeout…).
    pub error: Option<String>,
    pub plugins: Vec<PluginInfo>,
    pub scanned_at: String,
    /// The bundle no longer exists on disk.
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCatalog {
    /// Whether the helper process was found (hosting possible).
    pub available: bool,
    pub helper_path: Option<String>,
    pub default_folders: Vec<String>,
    pub user_folders: Vec<String>,
    pub bundles: Vec<PluginBundle>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginParams {
    pub params: Vec<ParamDesc>,
    pub values: Vec<ParamValue>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub bundles: usize,
    pub scanned: usize,
    pub unchanged: usize,
    pub failed: usize,
    pub effects: usize,
}

fn outcome_of(e: &PluginCacheEntry) -> Option<ScanOutcome> {
    serde_json::from_value(e.data.clone()).ok()
}

impl AppCore {
    /// Point plugin hosting at a specific helper executable (tests,
    /// unusual installs). `None` disables hosting.
    pub fn set_plugin_helper(&self, helper: Option<PathBuf>) {
        let mut pm = self.plugins.lock();
        pm.shutdown_all();
        *pm = uta_plugins::PluginManager::new(helper);
        drop(pm);
        let _ = self.refresh_plugin_catalog();
    }

    fn plugin_folders(&self) -> Vec<PathBuf> {
        let mut f = default_folders();
        f.extend(self.settings().plugin_folders.iter().map(PathBuf::from));
        f
    }

    /// Feed the scan cache (installed plugins, disabled bundles) to the
    /// live plugin manager.
    pub fn refresh_plugin_catalog(&self) -> Result<()> {
        let entries = self.library.lock().plugin_cache()?;
        let mut known = HashMap::new();
        let mut disabled = HashSet::new();
        for e in &entries {
            if !e.enabled {
                disabled.insert(e.path.clone());
            }
            if let Some(o) = outcome_of(e) {
                for p in o.plugins {
                    known.entry(p.uid).or_insert_with(|| PathBuf::from(&e.path));
                }
            }
        }
        self.plugins.lock().set_catalog(known, disabled);
        Ok(())
    }

    pub fn plugin_catalog(&self) -> Result<PluginCatalog> {
        let entries = self.library.lock().plugin_cache()?;
        let helper = self.plugins.lock().helper().map(|p| p.to_string_lossy().to_string());
        let bundles = entries
            .iter()
            .map(|e| {
                let o = outcome_of(e);
                PluginBundle {
                    path: e.path.clone(),
                    enabled: e.enabled,
                    error: o.as_ref().and_then(|o| o.error.clone()),
                    plugins: o.map(|o| o.plugins).unwrap_or_default(),
                    scanned_at: e.scanned_at.to_rfc3339(),
                    missing: !PathBuf::from(&e.path).exists(),
                }
            })
            .collect();
        Ok(PluginCatalog {
            available: helper.is_some(),
            helper_path: helper,
            default_folders: default_folders().iter().map(|p| p.to_string_lossy().to_string()).collect(),
            user_folders: self.settings().plugin_folders,
            bundles,
        })
    }

    /// Scan the plugin folders in the background. Each bundle is loaded in
    /// its own helper process, so a plugin that crashes or hangs during the
    /// scan is only marked as failed. Unchanged bundles are skipped unless
    /// `force` is set.
    pub fn plugin_scan(self: &Arc<Self>, force: bool) -> Result<u64> {
        let helper = self
            .plugins
            .lock()
            .helper()
            .map(PathBuf::from)
            .ok_or_else(|| CoreError::Invalid("plugin hosting is unavailable: the uta-plugin-host helper was not found next to the app".into()))?;
        let folders = self.plugin_folders();
        let core = self.clone();
        Ok(self.jobs.submit("plugin-scan", "Scanning plugins", move |job| {
            let bundles = discover(&folders);
            let cached: HashMap<String, PluginCacheEntry> = core
                .library
                .lock()
                .plugin_cache()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|e| (e.path.clone(), e))
                .collect();
            let mut summary = ScanSummary { bundles: bundles.len(), scanned: 0, unchanged: 0, failed: 0, effects: 0 };
            for (i, b) in bundles.iter().enumerate() {
                if !job.report(i as f32 / bundles.len().max(1) as f32) {
                    return Err("cancelled".into());
                }
                let key = b.to_string_lossy().to_string();
                let modified = uta_plugins::scan::bundle_modified(b);
                if let Some(c) = cached.get(&key) {
                    if !force && c.modified == modified {
                        summary.unchanged += 1;
                        if let Some(o) = outcome_of(c) {
                            summary.effects += o.plugins.iter().filter(|p| !p.is_instrument).count();
                            if o.error.is_some() {
                                summary.failed += 1;
                            }
                        }
                        continue;
                    }
                }
                let o = scan_bundle(&helper, b);
                summary.scanned += 1;
                if o.error.is_some() {
                    summary.failed += 1;
                }
                summary.effects += o.plugins.iter().filter(|p| !p.is_instrument).count();
                let entry = PluginCacheEntry {
                    path: key,
                    modified: o.modified,
                    data: serde_json::to_value(&o).map_err(|e| e.to_string())?,
                    enabled: cached.get(&o.path).map(|c| c.enabled).unwrap_or(true),
                    scanned_at: crate::now(),
                };
                core.library.lock().upsert_plugin_cache(&entry).map_err(|e| e.to_string())?;
            }
            // forget bundles that were uninstalled from scanned folders
            let present: HashSet<String> = bundles.iter().map(|b| b.to_string_lossy().to_string()).collect();
            for path in cached.keys() {
                let in_scanned_folder = folders.iter().any(|f| PathBuf::from(path).starts_with(f));
                if in_scanned_folder && !present.contains(path) {
                    let _ = core.library.lock().remove_plugin_cache(path);
                }
            }
            core.refresh_plugin_catalog().map_err(|e| e.to_string())?;
            core.plugins.lock().clear_failures();
            core.plugin_resync();
            serde_json::to_value(summary).map_err(|e| e.to_string())
        }))
    }

    /// Enable or disable a bundle. Disabling unloads it from the open
    /// project (its slots pass audio through and keep their settings).
    pub fn plugin_set_enabled(&self, path: &str, enabled: bool) -> Result<()> {
        self.library.lock().set_plugin_enabled(path, enabled)?;
        self.refresh_plugin_catalog()?;
        {
            let mut pm = self.plugins.lock();
            pm.unload_path(std::path::Path::new(path));
            pm.clear_failures();
        }
        self.plugin_resync();
        Ok(())
    }

    /// Remove a bundle from the list (it is found again by the next scan if
    /// still installed).
    pub fn plugin_forget(&self, path: &str) -> Result<()> {
        self.library.lock().remove_plugin_cache(path)?;
        self.refresh_plugin_catalog()
    }

    /// Rebuild the open project's audio graph so plugin changes apply.
    fn plugin_resync(&self) {
        let mut studio = self.studio.lock();
        if let Some(s) = studio.as_mut() {
            let mut audio = self.audio.lock();
            if let Some(e) = audio.engine.as_mut() {
                s.sync_engine(e, true, &mut self.plugins.lock());
            }
        }
    }

    fn plugin_slot_check(&self, track: Option<Id>, slot: Id) -> Result<()> {
        let studio = self.studio.lock();
        let s = studio.as_ref().ok_or(CoreError::NoProject)?;
        let chain = match track {
            Some(t) => &s.project.track(t).ok_or_else(|| CoreError::NotFound("track".into()))?.effects,
            None => &s.project.master.effects,
        };
        match chain.iter().find(|x| x.id == slot) {
            Some(x) if matches!(x.effect, EffectKind::Plugin { .. }) => Ok(()),
            Some(_) => Err(CoreError::Invalid("that slot is a built-in effect".into())),
            None => Err(CoreError::NotFound("effect slot".into())),
        }
    }

    /// Parameter list and current values of a loaded plugin slot.
    pub fn studio_plugin_params(&self, track: Option<Id>, slot: Id) -> Result<PluginParams> {
        self.plugin_slot_check(track, slot)?;
        let pm = self.plugins.lock();
        let params = pm.param_descs(slot).map_err(|e| CoreError::Invalid(e.to_string()))?;
        let values = pm.param_values(slot).map_err(|e| CoreError::Invalid(e.to_string()))?;
        Ok(PluginParams { params, values })
    }

    /// Restart a crashed (or failed) plugin from its last saved state plus
    /// the parameter values stored in the project.
    pub fn studio_reload_plugin(&self, track: Option<Id>, slot: Id) -> Result<crate::studio::StudioState> {
        self.plugin_slot_check(track, slot)?;
        self.plugins.lock().reset_slot(slot);
        self.plugin_resync();
        self.studio_state()?.ok_or(CoreError::NoProject)
    }

    /// Test hook: crash the helper behind a slot.
    #[doc(hidden)]
    pub fn debug_crash_plugin(&self, slot: Id) {
        self.plugins.lock().crash_for_testing(slot);
    }
}
