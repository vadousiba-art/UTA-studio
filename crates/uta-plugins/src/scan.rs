//! Finding VST3 bundles and scanning them, each in a throwaway helper.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uta_plugin_protocol::{PluginInfo, Request, Response};

use crate::client::{ClientError, HelperClient};

/// Time allowed for loading one module and listing its classes.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(30);

/// The standard VST3 locations for this platform (only those that exist
/// are scanned). See the VST3 documentation "Plug-in Locations".
pub fn default_folders() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(windows)]
    {
        if let Some(c) = std::env::var_os("CommonProgramFiles") {
            v.push(PathBuf::from(c).join("VST3"));
        } else {
            v.push(PathBuf::from(r"C:\Program Files\Common Files\VST3"));
        }
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(l).join("Programs").join("Common").join("VST3"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(h) = std::env::var_os("HOME") {
            v.push(PathBuf::from(h).join(".vst3"));
        }
        v.push(PathBuf::from("/usr/lib/vst3"));
        v.push(PathBuf::from("/usr/local/lib/vst3"));
    }
    v
}

fn is_vst3(p: &Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("vst3")).unwrap_or(false)
}

/// All `.vst3` bundles (or single-file modules) under `folders`, sorted.
/// Bundles are not descended into; depth is limited to avoid link loops.
pub fn discover(folders: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth > 8 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if is_vst3(&p) {
                out.push(p);
            } else if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk(&p, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    for f in folders {
        if is_vst3(f) {
            out.push(f.clone());
        } else {
            walk(f, 0, &mut out);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Modification time (seconds) of a bundle, used to skip unchanged bundles
/// on rescan. Uses the newest of the bundle and its binary directory.
pub fn bundle_modified(path: &Path) -> i64 {
    let secs = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    };
    let mut m = secs(path);
    let contents = path.join("Contents");
    if contents.is_dir() {
        if let Ok(rd) = std::fs::read_dir(&contents) {
            for e in rd.flatten() {
                m = m.max(secs(&e.path()));
                if let Ok(rd2) = std::fs::read_dir(e.path()) {
                    for f in rd2.flatten() {
                        m = m.max(secs(&f.path()));
                    }
                }
            }
        }
    }
    m
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanOutcome {
    pub path: String,
    pub modified: i64,
    /// Audio effect classes found (instruments are listed with
    /// `is_instrument` and are not offered for insertion).
    pub plugins: Vec<PluginInfo>,
    pub error: Option<String>,
}

/// Scan one bundle in a fresh helper process.
pub fn scan_bundle(helper: &Path, path: &Path) -> ScanOutcome {
    let modified = bundle_modified(path);
    let mut out = ScanOutcome { path: path.to_string_lossy().to_string(), modified, plugins: Vec::new(), error: None };
    let result = HelperClient::spawn(helper).and_then(|mut c| {
        let r = c.request(&Request::Scan { path: out.path.clone() }, SCAN_TIMEOUT);
        c.shutdown();
        r
    });
    match result {
        Ok(Response::Scanned { plugins }) => {
            if plugins.is_empty() {
                out.error = Some("this bundle contains no audio effects".into());
            }
            out.plugins = plugins;
        }
        Ok(r) => out.error = Some(format!("unexpected reply {r:?}")),
        Err(ClientError::Plugin(m)) => out.error = Some(m),
        Err(e) => out.error = Some(e.to_string()),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_bundles_without_descending() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("Vendor").join("A.vst3");
        std::fs::create_dir_all(a.join("Contents").join("x86_64-win")).unwrap();
        std::fs::write(d.path().join("B.VST3"), b"x").unwrap();
        std::fs::write(d.path().join("readme.txt"), b"x").unwrap();
        let found = discover(&[d.path().to_path_buf(), d.path().join("missing")]);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().any(|p| p.ends_with("A.vst3")));
        assert!(bundle_modified(&a) > 0);
    }
}
