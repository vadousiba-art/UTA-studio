//! User settings and data locations.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uta_coach::exercises::VoiceRange;

/// Where everything lives. All under the user's own folders; nothing is
/// stored elsewhere and nothing is uploaded.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPaths {
    /// App data: library database, practice recordings, Song Lab songs.
    pub data_dir: PathBuf,
    pub library_db: PathBuf,
    pub recordings_dir: PathBuf,
    pub songs_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub logs_dir: PathBuf,
    /// Default folder for new studio projects.
    pub projects_dir: PathBuf,
}

impl AppPaths {
    pub fn from_root(data_dir: &Path, projects_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            library_db: data_dir.join("library.sqlite"),
            recordings_dir: data_dir.join("recordings"),
            songs_dir: data_dir.join("songs"),
            backups_dir: data_dir.join("backups"),
            logs_dir: data_dir.join("logs"),
            projects_dir: projects_dir.to_path_buf(),
        }
    }

    /// Platform defaults: `%APPDATA%\UTA Vocal Lab` and
    /// `Documents\UTA Vocal Lab\Projects` on Windows.
    pub fn platform_default() -> Self {
        let data = dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("UTA Vocal Lab");
        let docs = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("UTA Vocal Lab")
            .join("Projects");
        Self::from_root(&data, &docs)
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for d in [&self.data_dir, &self.recordings_dir, &self.songs_dir, &self.backups_dir, &self.logs_dir, &self.projects_dir] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionLength {
    Short,
    Long,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// The user explicitly allowed microphone access (first-run consent).
    pub microphone_enabled: bool,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub sample_rate: Option<u32>,
    pub buffer_frames: Option<u32>,
    /// Input channel used by the Analyzer and Coach (0-based).
    pub input_channel: u16,
    /// Manual recording-latency correction from calibration (ms, ±).
    pub latency_offset_ms: f32,
    pub voice_range: VoiceRange,
    pub voice_range_measured: bool,
    pub show_romaji: bool,
    pub show_kana: bool,
    /// Analysis resolution (hop) in milliseconds: 10 = detailed, 20 = light.
    pub analysis_hop_ms: f32,
    /// Lightweight mode: no live pitch when not needed, coarser analysis,
    /// smaller waveform resolution.
    pub lightweight_mode: bool,
    pub autosave_minutes: u32,
    pub session_length: SessionLength,
    /// Extra folders to scan for VST3 plugins.
    pub plugin_folders: Vec<String>,
    pub reference_volume: f32,
    pub monitor_in_analyzer: bool,
    pub first_run_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            microphone_enabled: false,
            input_device: None,
            output_device: None,
            sample_rate: None,
            buffer_frames: None,
            input_channel: 0,
            latency_offset_ms: 0.0,
            voice_range: VoiceRange::default(),
            voice_range_measured: false,
            show_romaji: true,
            show_kana: true,
            analysis_hop_ms: 10.0,
            lightweight_mode: false,
            autosave_minutes: 2,
            session_length: SessionLength::Short,
            plugin_folders: Vec::new(),
            reference_volume: 0.25,
            monitor_in_analyzer: false,
            first_run_done: false,
        }
    }
}

impl Settings {
    pub fn sanitized(mut self) -> Self {
        self.voice_range = self.voice_range.sanitized();
        self.analysis_hop_ms = self.analysis_hop_ms.clamp(5.0, 40.0);
        self.autosave_minutes = self.autosave_minutes.clamp(1, 30);
        self.latency_offset_ms = self.latency_offset_ms.clamp(-500.0, 500.0);
        self.reference_volume = self.reference_volume.clamp(0.0, 1.0);
        if let Some(b) = self.buffer_frames {
            self.buffer_frames = Some(b.clamp(32, 8192));
        }
        self
    }
}
