//! Application core for UTA Vocal Lab.
//!
//! [`AppCore`] owns the audio engine, the library database, the open studio
//! project and background jobs, and exposes every user-facing operation as a
//! plain method. The Tauri layer only forwards IPC calls to it, so complete
//! workflows are tested headlessly (see `tests/`).
//!
//! Lock order (to avoid deadlocks): `studio` → `audio` → `plugins` →
//! `capture` → `library` → `settings`.

pub mod analyzer;
pub mod coach;
pub mod error;
pub mod jobs;
pub mod live;
pub mod plugins;
pub mod presets;
pub mod romaji;
pub mod settings;
pub mod songlab;
pub mod studio;

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use uta_audio::device::{BackendInfo, DeviceList, DeviceSelection};
use uta_audio::{AudioEngine, BackendChoice, EngineStatus};
use uta_project::Library;

pub use error::{CoreError, ErrorPayload, Result};
pub use settings::{AppPaths, Settings};

use crate::jobs::Jobs;
use crate::live::{LiveFrame, LiveWorker};

/// Chooses the audio backend. Production uses the real device; tests inject
/// the null backend with synthetic input.
pub type BackendFactory = Box<dyn Fn(&Settings) -> BackendChoice + Send + Sync>;

pub(crate) struct AudioState {
    pub engine: Option<AudioEngine>,
    pub live: Option<LiveWorker>,
    pub last_error: Option<String>,
}

/// What is currently being captured outside the studio timeline.
#[derive(Debug, Clone)]
pub(crate) enum CaptureKind {
    Analyzer,
    Coach(Box<uta_coach::exercises::ExercisePlan>),
    RangeLow,
    RangeHigh,
    Song(Box<songlab::SongCapture>),
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveCapture {
    pub kind: CaptureKind,
    pub path: PathBuf,
    pub started: chrono::DateTime<chrono::Utc>,
    pub latency_s: f32,
}

pub struct AppCore {
    pub paths: AppPaths,
    pub(crate) library: Mutex<Library>,
    pub(crate) settings: Mutex<Settings>,
    pub(crate) audio: Mutex<AudioState>,
    pub(crate) capture: Mutex<Option<ActiveCapture>>,
    pub(crate) studio: Mutex<Option<studio::StudioSession>>,
    pub(crate) plugins: Mutex<uta_plugins::PluginManager>,
    pub jobs: Arc<Jobs>,
    backend: BackendFactory,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub platform: String,
    pub paths: AppPaths,
    pub settings: Settings,
    pub disclaimer: String,
    pub engine: Option<BackendInfo>,
    pub engine_error: Option<String>,
}

const SETTINGS_KEY: &str = "app_settings";

impl AppCore {
    /// Production constructor: real audio devices, platform data folders.
    pub fn new(paths: AppPaths) -> Result<Self> {
        Self::with_backend(
            paths,
            Box::new(|s: &Settings| {
                BackendChoice::Device(DeviceSelection {
                    output: s.output_device.clone(),
                    input: s.input_device.clone(),
                    sample_rate: s.sample_rate,
                    buffer_frames: s.buffer_frames,
                    input_enabled: s.microphone_enabled,
                })
            }),
        )
    }

    pub fn with_backend(paths: AppPaths, backend: BackendFactory) -> Result<Self> {
        paths.ensure()?;
        let library = Library::open(&paths.library_db)?;
        let settings: Settings = library.get_setting(SETTINGS_KEY)?.unwrap_or_default();
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).saturating_sub(1).clamp(1, 2);
        Ok(Self {
            paths,
            library: Mutex::new(library),
            settings: Mutex::new(settings.sanitized()),
            audio: Mutex::new(AudioState { engine: None, live: None, last_error: None }),
            capture: Mutex::new(None),
            studio: Mutex::new(None),
            plugins: Mutex::new(uta_plugins::PluginManager::new(uta_plugins::locate_helper())),
            jobs: Arc::new(Jobs::new(workers)),
            backend,
        })
        .inspect(|core: &Self| {
            if let Err(e) = core.refresh_plugin_catalog() {
                tracing::warn!("plugin catalog unavailable: {e}");
            }
        })
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().clone()
    }

    /// Replace settings. Device-related changes restart the engine.
    pub fn update_settings(&self, new: Settings) -> Result<Settings> {
        let new = new.sanitized();
        let old = self.settings();
        self.library.lock().set_setting(SETTINGS_KEY, &new)?;
        *self.settings.lock() = new.clone();
        let device_changed = old.microphone_enabled != new.microphone_enabled
            || old.input_device != new.input_device
            || old.output_device != new.output_device
            || old.sample_rate != new.sample_rate
            || old.buffer_frames != new.buffer_frames;
        if device_changed {
            self.restart_engine()?;
        } else if let Some(e) = self.audio.lock().engine.as_mut() {
            e.manual_latency_frames = (new.latency_offset_ms / 1000.0 * e.sample_rate() as f32) as i64;
        }
        Ok(new)
    }

    pub fn info(&self) -> AppInfo {
        let audio = self.audio.lock();
        AppInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            paths: self.paths.clone(),
            settings: self.settings(),
            disclaimer: uta_coach::DISCLAIMER.into(),
            engine: audio.engine.as_ref().map(|e| e.info().clone()),
            engine_error: audio.last_error.clone(),
        }
    }

    pub fn list_devices(&self) -> Result<DeviceList> {
        Ok(uta_audio::device::list_devices()?)
    }

    /// Start (or restart) the audio engine from the current settings. If the
    /// device cannot be opened, falls back to the silent null backend so the
    /// app stays usable, and records the error for the UI.
    pub fn start_engine(&self) -> Result<BackendInfo> {
        {
            let a = self.audio.lock();
            if let Some(e) = a.engine.as_ref() {
                return Ok(e.info().clone());
            }
        }
        self.restart_engine()
    }

    pub fn restart_engine(&self) -> Result<BackendInfo> {
        if self.capture.lock().is_some() {
            return Err(CoreError::Busy("finish the current recording first".into()));
        }
        let settings = self.settings();
        let mut studio = self.studio.lock();
        if let Some(s) = studio.as_ref() {
            if s.recording.is_some() {
                return Err(CoreError::Busy("stop recording first".into()));
            }
        }
        let mut audio = self.audio.lock();
        audio.live = None;
        audio.engine = None; // drop the old streams before opening new ones
        let choice = (self.backend)(&settings);
        let (mut engine, err) = match AudioEngine::start(choice) {
            Ok(e) => (e, None),
            Err(e) => {
                tracing::error!("audio device failed to open: {e}");
                let fallback = AudioEngine::start(BackendChoice::Null(uta_audio::NullOptions::new(settings.sample_rate.unwrap_or(48_000))))?;
                (fallback, Some(format!("{e}. Audio is running silently — choose a working device in Settings.")))
            }
        };
        engine.manual_latency_frames = (settings.latency_offset_ms / 1000.0 * engine.sample_rate() as f32) as i64;
        let sr = engine.sample_rate();
        let hop = if settings.lightweight_mode { 20.0 } else { settings.analysis_hop_ms };
        let live = engine.take_analysis_stream().map(|ring| LiveWorker::spawn(ring, sr, hop));
        let _ = engine.set_analysis_channel(settings.input_channel);
        let info = engine.info().clone();
        audio.engine = Some(engine);
        audio.live = live;
        audio.last_error = err.or_else(|| (!info.notes.is_empty()).then(|| info.notes.join(" ")));
        // reload the open project into the new engine
        if let Some(s) = studio.as_mut() {
            if let Some(e) = audio.engine.as_mut() {
                s.sync_engine(e, true, &mut self.plugins.lock());
            }
        }
        Ok(info)
    }

    pub(crate) fn with_engine<T>(&self, f: impl FnOnce(&mut AudioEngine) -> Result<T>) -> Result<T> {
        let mut a = self.audio.lock();
        let e = a.engine.as_mut().ok_or(CoreError::Audio(uta_audio::AudioError::NotRunning))?;
        f(e)
    }

    pub fn engine_status(&self) -> Result<EngineStatus> {
        self.with_engine(|e| Ok(e.status()))
    }

    pub fn reset_clip_indicators(&self) -> Result<()> {
        self.with_engine(|e| {
            e.reset_clip_indicators();
            Ok(())
        })
    }

    pub fn set_live_pitch(&self, on: bool) {
        if let Some(l) = self.audio.lock().live.as_ref() {
            l.set_enabled(on);
        }
    }

    pub fn live_pitch(&self, since: u64) -> Vec<LiveFrame> {
        self.audio
            .lock()
            .live
            .as_ref()
            .map(|l| l.frames_since(since, 400))
            .unwrap_or_default()
    }

    pub fn play_reference(&self, midi: f32, seconds: f32) -> Result<()> {
        if !(20.0..=110.0).contains(&midi) {
            return Err(CoreError::Invalid("note out of range".into()));
        }
        let amp = self.settings().reference_volume;
        self.with_engine(|e| Ok(e.play_tone(uta_dsp::tone::midi_to_hz(midi), amp, seconds.clamp(0.1, 10.0))?))
    }

    pub fn stop_reference(&self) -> Result<()> {
        self.with_engine(|e| {
            e.stop_tone()?;
            e.stop_sequence()?;
            Ok(())
        })
    }

    /// Play a WAV file from the library/project through the engine.
    pub fn preview_file(&self, path: PathBuf, start_seconds: f32) -> Result<u64> {
        let managed = path.starts_with(&self.paths.data_dir)
            || self.studio.lock().as_ref().map(|s| path.starts_with(s.store.dir())).unwrap_or(false);
        if !managed {
            return Err(CoreError::Invalid("only recordings managed by the app can be previewed".into()));
        }
        if !path.is_file() {
            return Err(CoreError::NotFound(path.display().to_string()));
        }
        let info = uta_audio::wav::wav_info(&path)?;
        let start = (start_seconds.max(0.0) * info.sample_rate as f32) as u64;
        self.with_engine(|e| {
            if info.sample_rate != e.sample_rate() {
                return Err(CoreError::Invalid(format!(
                    "this file is {} Hz but the audio device runs at {} Hz",
                    info.sample_rate,
                    e.sample_rate()
                )));
            }
            Ok(e.preview(path, start, 1.0)?)
        })
    }

    pub fn stop_preview(&self) -> Result<()> {
        self.with_engine(|e| Ok(e.stop_preview()?))
    }

    /// Delete all practice history (attempts, skill levels, reviews, notes).
    pub fn delete_training_history(&self) -> Result<()> {
        Ok(self.library.lock().delete_training_history()?)
    }

    /// Back up the library database to the backups folder; returns the path.
    pub fn backup_library(&self) -> Result<PathBuf> {
        let dest = self
            .paths
            .backups_dir
            .join(format!("library-{}.sqlite", chrono::Utc::now().format("%Y%m%d-%H%M%S")));
        self.library.lock().backup_to(&dest)?;
        Ok(dest)
    }

    /// Stop everything cleanly (window closing).
    pub fn shutdown(&self) {
        let _ = self.studio_autosave_if_dirty();
        let mut a = self.audio.lock();
        if let Some(e) = a.engine.as_mut() {
            if e.is_recording() {
                let _ = e.stop_recording();
            }
            let _ = e.stop();
        }
        a.live = None;
        a.engine = None;
        drop(a);
        self.plugins.lock().shutdown_all();
    }
}

pub(crate) fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}
