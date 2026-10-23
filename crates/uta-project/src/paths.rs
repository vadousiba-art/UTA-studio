//! Path validation for project-relative asset paths.

use std::path::{Component, Path, PathBuf};

use crate::error::StoreError;

/// Folder inside a project that holds audio assets.
pub const AUDIO_DIR: &str = "audio";
/// Folder for regenerable caches (waveform peaks, stretched previews).
pub const CACHE_DIR: &str = "cache";
/// The SQLite document store inside a project folder.
pub const PROJECT_DB: &str = "project.utadb";
/// Project folders end with this extension.
pub const PROJECT_EXT: &str = "utaproj";

/// Validate a stored relative asset path (`audio/<name>`) and resolve it under
/// `project_dir`. Rejects absolute paths, `..`, drive prefixes, backslashes
/// and anything outside the audio folder, so a crafted project file cannot
/// make the app read or write elsewhere on disk.
pub fn resolve_asset_path(project_dir: &Path, rel: &str) -> Result<PathBuf, StoreError> {
    if rel.is_empty() || rel.contains('\\') || rel.contains('\0') || rel.len() > 512 {
        return Err(StoreError::UnsafePath(rel.to_string()));
    }
    let p = Path::new(rel);
    let mut comps = p.components();
    match comps.next() {
        Some(Component::Normal(first)) if first == AUDIO_DIR => {}
        _ => return Err(StoreError::UnsafePath(rel.to_string())),
    }
    let mut count = 0;
    for c in comps {
        match c {
            Component::Normal(_) => count += 1,
            _ => return Err(StoreError::UnsafePath(rel.to_string())),
        }
    }
    if count == 0 {
        return Err(StoreError::UnsafePath(rel.to_string()));
    }
    Ok(project_dir.join(p))
}

/// Make a safe file name from user input (keeps letters incl. Japanese, digits,
/// space, dash, underscore; everything else becomes `_`).
pub fn sanitize_file_stem(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
        .collect();
    let s = s.trim().trim_matches('.').to_string();
    let s: String = s.chars().take(60).collect();
    if s.is_empty() {
        "untitled".to_string()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_audio_paths() {
        let d = Path::new("/proj");
        assert_eq!(resolve_asset_path(d, "audio/take 1.wav").unwrap(), Path::new("/proj/audio/take 1.wav"));
    }

    #[test]
    fn rejects_escapes() {
        let d = Path::new("/proj");
        for bad in ["../x.wav", "audio/../../etc/passwd", "/etc/passwd", "audio", "cache/x.wav", "audio\\..\\x", "C:/x.wav", ""] {
            assert!(resolve_asset_path(d, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize_file_stem("My Song: 歌/../x"), "My Song_ 歌____x");
        assert_eq!(sanitize_file_stem("..."), "___");
        assert_eq!(sanitize_file_stem("  "), "untitled");
    }
}
