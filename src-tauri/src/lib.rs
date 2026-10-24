//! Tauri shell for UTA Vocal Lab. All application logic lives in the
//! `uta-*` crates; this crate only exposes `uta_core::AppCore` over IPC.
//!
//! Every command is `async` and runs its work on Tauri's blocking pool, so the
//! UI thread never waits on audio, disk or analysis.

mod commands;

use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;
use uta_core::{AppCore, AppPaths};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let paths = AppPaths::platform_default();
    let _ = std::fs::create_dir_all(&paths.logs_dir);
    init_logging(&paths);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let core = match AppCore::new(paths.clone()) {
                Ok(c) => Arc::new(c),
                Err(e) => {
                    tracing::error!("could not initialise app data at {}: {e}", paths.data_dir.display());
                    return Err(Box::new(std::io::Error::other(format!("UTA Vocal Lab could not open its data folder: {e}"))));
                }
            };
            if let Err(e) = core.start_engine() {
                tracing::error!("audio engine failed to start: {e}");
            }
            // autosave ticker
            let c = core.clone();
            std::thread::Builder::new()
                .name("uta-autosave".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_secs(20));
                    if Arc::strong_count(&c) <= 1 {
                        return;
                    }
                    if let Err(e) = c.studio_autosave_if_dirty() {
                        tracing::warn!("autosave failed: {e}");
                    }
                })
                .ok();
            app.manage(core);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(core) = window.try_state::<Arc<AppCore>>() {
                    core.shutdown();
                }
            }
        })
        .invoke_handler(commands::handler())
        .run(tauri::generate_context!())
        .expect("error while running UTA Vocal Lab");
}

fn init_logging(paths: &AppPaths) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.logs_dir.join("uta-vocal-lab.log"));
    match log_file {
        Ok(f) => {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(f))
                .try_init();
        }
        Err(_) => {
            let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
        }
    }
}
