//! Complete analysis of one recorded phrase.

use serde::{Deserialize, Serialize};

use crate::compare::{compare_to_targets, TargetComparison, TargetNote};
use crate::dynamics::{analyze_dynamics, DynamicsSummary};
use crate::notes::{analyze_sustain, segment_notes, DetectedNote, SustainAnalysis};
use crate::pitch::{PitchConfig, PitchFrame, PitchTracker};
use crate::quality::{assess_quality, RecordingQuality};
use crate::spectral::{analyze_spectrum, SpectralSummary};
use crate::stats::{mean, percentile};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisOptions {
    /// Notes the singer was asked to sing (empty = free singing).
    #[serde(default)]
    pub targets: Vec<TargetNote>,
    /// Accept the right note in a different octave (pitch-matching a
    /// reference tone outside the singer's range).
    #[serde(default)]
    pub octave_tolerant: bool,
    /// Analysis hop in seconds; larger = faster and coarser.
    #[serde(default = "default_hop")]
    pub hop_seconds: f32,
}

fn default_hop() -> f32 {
    0.01
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            octave_tolerant: false,
            hop_seconds: default_hop(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitchSummary {
    pub voiced_ratio: f32,
    pub voiced_seconds: f32,
    pub median_midi: Option<f32>,
    /// 5th / 95th percentile pitch — the range actually used.
    pub low_midi: Option<f32>,
    pub high_midi: Option<f32>,
    pub mean_confidence: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhraseAnalysis {
    /// Version of the analysis algorithms; stored with results so old results
    /// can be told apart after algorithm changes.
    pub analysis_version: u32,
    pub duration: f32,
    pub sample_rate: u32,
    pub hop_seconds: f32,
    pub frames: Vec<PitchFrame>,
    pub notes: Vec<DetectedNote>,
    pub pitch: PitchSummary,
    pub dynamics: DynamicsSummary,
    pub spectral: SpectralSummary,
    /// Analysis of the longest held note (if any note lasted ≥ 0.5 s).
    pub sustain: Option<SustainAnalysis>,
    pub quality: RecordingQuality,
    pub targets: Option<TargetComparison>,
}

pub const ANALYSIS_VERSION: u32 = 1;

/// Analyse a mono phrase.
pub fn analyze_phrase(samples: &[f32], sample_rate: u32, options: &AnalysisOptions) -> PhraseAnalysis {
    let mut cfg = PitchConfig::new(sample_rate);
    cfg.hop_seconds = options.hop_seconds.clamp(0.005, 0.05);
    let frames = PitchTracker::new(cfg).track(samples);
    let duration = samples.len() as f32 / sample_rate as f32;

    let notes = segment_notes(&frames, 0.08, 0.05);
    let dynamics = analyze_dynamics(samples, &frames);
    let (_, spectral) = analyze_spectrum(samples, sample_rate, &frames);
    let quality = assess_quality(duration, &frames, &dynamics);

    let midis: Vec<f32> = frames.iter().filter_map(|f| f.midi()).collect();
    let confs: Vec<f32> = frames.iter().filter(|f| f.is_voiced()).map(|f| f.confidence).collect();
    let pitch = PitchSummary {
        voiced_ratio: if frames.is_empty() {
            0.0
        } else {
            midis.len() as f32 / frames.len() as f32
        },
        voiced_seconds: midis.len() as f32 * cfg.hop_seconds,
        median_midi: percentile(&midis, 50.0),
        low_midi: percentile(&midis, 5.0),
        high_midi: percentile(&midis, 95.0),
        mean_confidence: mean(&confs),
    };

    let sustain = notes
        .iter()
        .filter(|n| n.duration() >= 0.5)
        .max_by(|a, b| a.duration().total_cmp(&b.duration()))
        .and_then(|n| {
            let target = options
                .targets
                .iter()
                .find(|t| t.start < n.end && t.end() > n.start)
                .map(|t| t.midi);
            let nf: Vec<PitchFrame> = frames
                .iter()
                .filter(|f| f.time >= n.start && f.time <= n.end)
                .copied()
                .collect();
            analyze_sustain(&nf, target)
        });

    let targets = if options.targets.is_empty() {
        None
    } else {
        Some(compare_to_targets(&frames, &options.targets, options.octave_tolerant))
    };

    PhraseAnalysis {
        analysis_version: ANALYSIS_VERSION,
        duration,
        sample_rate,
        hop_seconds: cfg.hop_seconds,
        frames,
        notes,
        pitch,
        dynamics,
        spectral,
        sustain,
        quality,
        targets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::QualityLevel;
    use crate::synth;

    #[test]
    fn clean_take_with_room_tone_is_good_quality() {
        let sr = 48_000;
        let mut sig = synth::noise(0.0005, 0.5, sr, 1); // ~ -70 dB room tone
        let tone = synth::harmonic_tone(220.0, &[1.0, 0.5, 0.3], 2.0, sr);
        sig.extend(tone.iter().map(|s| s * 0.4));
        sig.extend(synth::noise(0.0005, 0.5, sr, 2));
        let a = analyze_phrase(&sig, sr, &AnalysisOptions::default());
        assert_eq!(a.quality.level, QualityLevel::Good, "{:?}", a.quality);
        assert!(a.quality.snr_db.unwrap() > 40.0);
        let s = a.sustain.clone().expect("sustain");
        assert!((s.midi - 57.0).abs() < 0.05);
        assert!((a.pitch.voiced_seconds - 2.0).abs() < 0.1);
        // JSON round-trip
        let json = serde_json::to_string(&a).unwrap();
        let back: PhraseAnalysis = serde_json::from_str(&json).unwrap();
        assert_eq!(back.notes.len(), a.notes.len());
    }

    #[test]
    fn noisy_take_is_poor_quality() {
        let sr = 48_000;
        let noise = synth::noise(0.2, 3.0, sr, 5);
        let tone = synth::harmonic_tone(220.0, &[1.0, 0.5], 3.0, sr);
        // tone only in the middle second, noise everywhere at nearly equal level
        let sig: Vec<f32> = noise
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let t = i as f32 / sr as f32;
                n + if (1.0..2.0).contains(&t) { tone[i] * 0.25 } else { 0.0 }
            })
            .collect();
        let a = analyze_phrase(&sig, sr, &AnalysisOptions::default());
        assert_eq!(a.quality.level, QualityLevel::Poor, "{:?}", a.quality);
    }

    #[test]
    fn empty_input_does_not_panic() {
        let a = analyze_phrase(&[], 48_000, &AnalysisOptions::default());
        assert_eq!(a.quality.level, QualityLevel::Poor);
        assert!(a.frames.is_empty());
    }
}
