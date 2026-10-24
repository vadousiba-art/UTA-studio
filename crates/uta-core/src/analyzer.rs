//! Vocal Analyzer: record a phrase, analyse it, compare attempts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uta_analysis::compare::{compare_contours, ContourComparison};
use uta_analysis::phrase::{analyze_phrase, AnalysisOptions, PhraseAnalysis};
use uta_coach::evaluate::{free_singing_feedback, Feedback};
use uta_project::library::RecordingEntry;

use crate::error::{CoreError, Result};
use crate::{now, ActiveCapture, AppCore, CaptureKind};

/// Longest analyzer/coach take (protects memory and keeps analysis fast).
pub const MAX_CAPTURE_SECONDS: u64 = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzerResult {
    pub recording: RecordingEntry,
    pub analysis: PhraseAnalysis,
    pub feedback: Feedback,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    pub active: bool,
    pub kind: Option<String>,
    pub seconds: f32,
    pub input_device: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricDelta {
    pub key: String,
    pub label: String,
    pub a: Option<f32>,
    pub b: Option<f32>,
    /// "better", "worse", "same" or "n/a" — only for metrics where a
    /// direction is meaningful (steadiness, drift…). Loudness and brightness
    /// are never ranked.
    pub verdict: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingComparison {
    pub a: RecordingEntry,
    pub b: RecordingEntry,
    pub contour: Option<ContourComparison>,
    pub metrics: Vec<MetricDelta>,
    pub notes: Vec<String>,
}

fn analysis_cache_path(wav: &Path) -> PathBuf {
    wav.with_extension("analysis.json")
}

pub(crate) fn summary_of(a: &PhraseAnalysis) -> serde_json::Value {
    serde_json::json!({
        "duration": a.duration,
        "voicedSeconds": a.pitch.voiced_seconds,
        "medianMidi": a.pitch.median_midi,
        "lowMidi": a.pitch.low_midi,
        "highMidi": a.pitch.high_midi,
        "quality": a.quality.level,
        "wobbleCents": a.sustain.as_ref().map(|s| s.wobble_cents),
        "driftCents": a.sustain.as_ref().map(|s| s.drift_cents),
        "dynamicRangeDb": a.dynamics.dynamic_range_db,
        "peakDb": a.dynamics.peak_db,
    })
}

impl AppCore {
    pub(crate) fn new_recording_path(&self, prefix: &str) -> Result<PathBuf> {
        let dir = self.paths.recordings_dir.join(now().format("%Y-%m").to_string());
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join(format!("{prefix}-{}-{}.wav", now().format("%Y%m%d-%H%M%S"), &uuid::Uuid::new_v4().simple().to_string()[..6])))
    }

    pub(crate) fn begin_capture(&self, kind: CaptureKind, guide: Option<(Vec<uta_audio::core::SeqNote>, Option<f32>)>) -> Result<()> {
        self.begin_capture_with(kind, guide, None)
    }

    /// Start a capture; `after` runs on the engine right after the capture
    /// command, under the same lock (e.g. to start backing playback).
    #[allow(clippy::type_complexity)]
    pub(crate) fn begin_capture_with(
        &self,
        kind: CaptureKind,
        guide: Option<(Vec<uta_audio::core::SeqNote>, Option<f32>)>,
        after: Option<Box<dyn FnOnce(&mut uta_audio::AudioEngine) -> Result<()>>>,
    ) -> Result<()> {
        if !self.settings().microphone_enabled {
            return Err(CoreError::Invalid("Microphone access is off. Enable it in Settings to record.".into()));
        }
        if let Some(s) = self.studio.lock().as_ref() {
            if s.recording.is_some() {
                return Err(CoreError::Busy("the studio is recording".into()));
            }
        }
        let mut cap = self.capture.lock();
        if cap.is_some() {
            return Err(CoreError::Busy("already recording".into()));
        }
        let prefix = match &kind {
            CaptureKind::Analyzer => "analyzer",
            CaptureKind::Coach(_) => "coach",
            CaptureKind::RangeLow | CaptureKind::RangeHigh => "range",
            CaptureKind::Song(_) => "song",
        };
        let path = self.new_recording_path(prefix)?;
        let channel = self.settings().input_channel;
        let latency_s = {
            let mut a = self.audio.lock();
            let e = a.engine.as_mut().ok_or(CoreError::Audio(uta_audio::AudioError::NotRunning))?;
            if !e.has_input() {
                return Err(CoreError::Audio(uta_audio::AudioError::NoDevice(
                    "no microphone is open — check the input device in Settings".into(),
                )));
            }
            let lat = (e.latency_compensation() as f32) / e.sample_rate() as f32;
            e.capture(path.clone(), channel, guide)?;
            if let Some(f) = after {
                if let Err(err) = f(e) {
                    let _ = e.stop_recording();
                    let _ = std::fs::remove_file(&path);
                    return Err(err);
                }
            }
            if let Some(l) = a.live.as_ref() {
                l.set_enabled(true);
            }
            lat
        };
        *cap = Some(ActiveCapture { kind, path, started: now(), latency_s });
        Ok(())
    }

    /// Stop the active capture; returns its info and the decoded mono audio.
    pub(crate) fn end_capture(&self) -> Result<(ActiveCapture, Vec<f32>, u32)> {
        let cap = self.capture.lock().take().ok_or_else(|| CoreError::Invalid("not recording".into()))?;
        let outcome = self.with_engine(|e| Ok(e.stop_recording()?))?;
        if !outcome.result.errors.is_empty() {
            tracing::warn!("recording errors: {:?}", outcome.result.errors);
        }
        let file = outcome
            .result
            .files
            .first()
            .ok_or_else(|| CoreError::Other(format!("the recording could not be saved: {}", outcome.result.errors.join("; "))))?;
        let buf = uta_audio::wav::read_wav(&file.path, Some(MAX_CAPTURE_SECONDS * outcome.sample_rate as u64 + outcome.sample_rate as u64))?;
        Ok((cap, buf.to_mono(), outcome.sample_rate))
    }

    pub fn capture_status(&self) -> CaptureStatus {
        let cap = self.capture.lock();
        let input = self.audio.lock().engine.as_ref().and_then(|e| e.info().input_name.clone());
        match cap.as_ref() {
            Some(c) => CaptureStatus {
                active: true,
                kind: Some(match &c.kind {
                    CaptureKind::Analyzer => "analyzer".into(),
                    CaptureKind::Coach(p) => format!("coach:{}", p.exercise_id),
                    CaptureKind::RangeLow => "rangeLow".into(),
                    CaptureKind::RangeHigh => "rangeHigh".into(),
                    CaptureKind::Song(s) => format!("song:{}", s.song),
                }),
                seconds: (now() - c.started).num_milliseconds() as f32 / 1000.0,
                input_device: input,
            },
            None => CaptureStatus { active: false, kind: None, seconds: 0.0, input_device: input },
        }
    }

    pub fn analyzer_start(&self) -> Result<()> {
        self.begin_capture(CaptureKind::Analyzer, None)
    }

    /// Cancel any capture without saving it.
    pub fn capture_cancel(&self) -> Result<()> {
        if self.capture.lock().is_none() {
            return Ok(());
        }
        let (cap, _, _) = self.end_capture()?;
        if matches!(cap.kind, CaptureKind::Song(_)) {
            let _ = self.stop_preview();
        }
        let _ = std::fs::remove_file(&cap.path);
        Ok(())
    }

    pub(crate) fn analysis_options(&self) -> AnalysisOptions {
        let s = self.settings();
        AnalysisOptions {
            targets: vec![],
            octave_tolerant: false,
            hop_seconds: if s.lightweight_mode { 0.02 } else { s.analysis_hop_ms / 1000.0 },
        }
    }

    pub fn analyzer_stop(&self) -> Result<AnalyzerResult> {
        let (cap, mono, sr) = self.end_capture()?;
        if !matches!(cap.kind, CaptureKind::Analyzer) {
            return Err(CoreError::Invalid("the active recording is not an analyzer take".into()));
        }
        let analysis = analyze_phrase(&mono, sr, &self.analysis_options());
        let feedback = free_singing_feedback(&analysis);
        let n = self.library.lock().recordings(1000, Some("analyzer"))?.len();
        let entry = RecordingEntry {
            id: uuid::Uuid::new_v4().to_string(),
            path: cap.path.to_string_lossy().to_string(),
            created_at: cap.started,
            duration: analysis.duration as f64,
            source: "analyzer".into(),
            title: format!("Take {}", n + 1),
            exercise_id: None,
            summary: summary_of(&analysis),
        };
        if let Ok(json) = serde_json::to_vec(&analysis) {
            let _ = std::fs::write(analysis_cache_path(&cap.path), json);
        }
        self.library.lock().add_recording(&entry)?;
        Ok(AnalyzerResult { recording: entry, analysis, feedback })
    }

    pub fn recordings(&self, source: Option<&str>, limit: usize) -> Result<Vec<RecordingEntry>> {
        Ok(self.library.lock().recordings(limit.min(1000), source)?)
    }

    pub fn recording(&self, id: &str) -> Result<RecordingEntry> {
        self.library.lock().recording(id)?.ok_or_else(|| CoreError::NotFound(format!("recording {id}")))
    }

    /// Load (or compute and cache) the analysis of a recording.
    pub fn recording_analysis(&self, id: &str) -> Result<AnalyzerResult> {
        let rec = self.recording(id)?;
        let path = PathBuf::from(&rec.path);
        let cache = analysis_cache_path(&path);
        let analysis = match std::fs::read(&cache).ok().and_then(|b| serde_json::from_slice::<PhraseAnalysis>(&b).ok()) {
            Some(a) if a.analysis_version == uta_analysis::phrase::ANALYSIS_VERSION => a,
            _ => {
                if !path.is_file() {
                    return Err(CoreError::NotFound(format!("audio file for {}", rec.title)));
                }
                let buf = uta_audio::wav::read_wav(&path, Some(MAX_CAPTURE_SECONDS * 192_000))?;
                let a = analyze_phrase(&buf.to_mono(), buf.sample_rate, &self.analysis_options());
                if let Ok(json) = serde_json::to_vec(&a) {
                    let _ = std::fs::write(&cache, json);
                }
                a
            }
        };
        let feedback = free_singing_feedback(&analysis);
        Ok(AnalyzerResult { recording: rec, analysis, feedback })
    }

    pub fn rename_recording(&self, id: &str, title: &str) -> Result<()> {
        let t = title.trim();
        if t.is_empty() {
            return Err(CoreError::Invalid("title cannot be empty".into()));
        }
        Ok(self.library.lock().rename_recording(id, &t.chars().take(80).collect::<String>())?)
    }

    pub fn delete_recording(&self, id: &str) -> Result<()> {
        if let Some(rec) = self.library.lock().recording(id)? {
            let _ = std::fs::remove_file(analysis_cache_path(Path::new(&rec.path)));
        }
        Ok(self.library.lock().delete_recording(id, &self.paths.recordings_dir)?)
    }

    pub fn compare_recordings(&self, a: &str, b: &str) -> Result<RecordingComparison> {
        let ra = self.recording_analysis(a)?;
        let rb = self.recording_analysis(b)?;
        let (x, y) = (&ra.analysis, &rb.analysis);
        let contour = if x.sample_rate > 0 && (x.hop_seconds - y.hop_seconds).abs() < 1e-6 {
            compare_contours(&y.frames, &x.frames, 2.0)
        } else {
            None
        };
        let mut metrics = Vec::new();
        let mut push = |key: &str, label: &str, va: Option<f32>, vb: Option<f32>, lower_better: Option<bool>| {
            let verdict = match (va, vb, lower_better) {
                (Some(p), Some(q), Some(lb)) => {
                    let d = q - p;
                    if d.abs() < 0.05 * p.abs().max(1.0) {
                        "same"
                    } else if (d < 0.0) == lb {
                        "better"
                    } else {
                        "worse"
                    }
                }
                _ => "n/a",
            };
            metrics.push(MetricDelta { key: key.into(), label: label.into(), a: va, b: vb, verdict: verdict.into() });
        };
        push("wobble", "Steadiness (wobble, ¢)", x.sustain.as_ref().map(|s| s.wobble_cents), y.sustain.as_ref().map(|s| s.wobble_cents), Some(true));
        push("drift", "Drift on longest note (¢)", x.sustain.as_ref().map(|s| s.drift_cents.abs()), y.sustain.as_ref().map(|s| s.drift_cents.abs()), Some(true));
        push("voiced", "Sung time (s)", Some(x.pitch.voiced_seconds), Some(y.pitch.voiced_seconds), None);
        push("median", "Middle of range (MIDI)", x.pitch.median_midi, y.pitch.median_midi, None);
        push("dynRange", "Dynamic range (dB)", x.dynamics.dynamic_range_db, y.dynamics.dynamic_range_db, None);
        push("brightness", "Brightness (Hz)", x.spectral.median_centroid_hz, y.spectral.median_centroid_hz, None);
        push("confidence", "Measurement confidence", x.pitch.mean_confidence, y.pitch.mean_confidence, None);
        let mut notes = vec!["Loudness, brightness and range are shown for comparison only — none of them is 'better' by itself.".to_string()];
        if x.quality.level != y.quality.level {
            notes.push("The two takes have different recording quality, so differences may partly come from the recording.".into());
        }
        Ok(RecordingComparison { a: ra.recording, b: rb.recording, contour, metrics, notes })
    }

    /// Waveform min/max pairs for any WAV the app manages.
    pub fn waveform(&self, path: &Path, start: u64, end: Option<u64>, buckets: usize) -> Result<Vec<f32>> {
        let allowed = path.starts_with(&self.paths.data_dir)
            || self.studio.lock().as_ref().map(|s| path.starts_with(s.store.dir())).unwrap_or(false);
        if !allowed {
            return Err(CoreError::Invalid("waveforms are only available for files managed by the app".into()));
        }
        let cache = path.parent().map(|p| p.join(".peaks")).unwrap_or_else(|| self.paths.data_dir.join(".peaks"));
        let peaks = uta_audio::waveform::Peaks::cached(path, &cache)?;
        Ok(peaks.range(start, end.unwrap_or(peaks.frames), buckets))
    }
}
