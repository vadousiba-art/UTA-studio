//! Test support: builds the helper binary and the UTA Test Gain fixture
//! plugin so integration tests in several crates can host a real VST3.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("workspace root")
}

/// Path of a freshly built `uta-plugin-host` (debug profile).
pub fn helper_binary() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let root = workspace_root();
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            let out = Command::new(cargo)
                .args(["build", "-q", "-p", "uta-plugin-host"])
                .current_dir(&root)
                .output()
                .expect("run cargo build");
            assert!(out.status.success(), "helper build failed: {}", String::from_utf8_lossy(&out.stderr));
            let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("target"));
            let p = target.join("debug").join(format!("uta-plugin-host{}", std::env::consts::EXE_SUFFIX));
            assert!(p.is_file(), "{} missing", p.display());
            p
        })
        .clone()
}

/// Path of the built `UTA Test Gain.vst3` bundle.
pub fn test_gain_bundle() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let root = workspace_root();
            let out = Command::new("bash")
                .arg(root.join("scripts/build-test-plugin.sh"))
                .output()
                .expect("run fixture build script");
            assert!(out.status.success(), "fixture build failed: {}", String::from_utf8_lossy(&out.stderr));
            PathBuf::from(String::from_utf8_lossy(&out.stdout).lines().last().expect("path").trim())
        })
        .clone()
}
