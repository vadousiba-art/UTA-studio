//! Level, clipping and phrase-level dynamics.
//!
//! Loudness here is plain RMS in dBFS of the recorded signal. It depends on
//! microphone gain and distance, so it is only meaningful *relative within a
//! take* (contrast, trend) — never as an absolute "how loud you sing" score.

use serde::{Deserialize, Serialize};

use crate::pitch::PitchFrame;
use crate::stats::{linear_slope, percentile};
use uta_dsp::level::gain_to_db;
use uta_dsp::meter::CLIP_THRESHOLD;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DynamicsSummary {
    /// Sample peak of the whole take (dBFS).
    pub peak_db: f32,
    /// Median RMS of voiced frames (dBFS); `None` if nothing was voiced.
    pub voiced_rms_db: Option<f32>,
    /// Number of samples at/over the clip threshold.
    pub clipped_samples: u64,
    /// Fraction of samples that clipped.
    pub clip_ratio: f32,
    /// Difference between loud (95th pct) and soft (10th pct) voiced frames.
    pub dynamic_range_db: Option<f32>,
    /// Linear trend of voiced-frame level in dB per second (+ = getting louder).
    pub trend_db_per_second: Option<f32>,
}

pub fn sample_peak_and_clips(samples: &[f32]) -> (f32, u64) {
    let mut peak = 0.0f32;
    let mut clips = 0u64;
    for &s in samples {
        let a = s.abs();
        peak = peak.max(a);
        if a >= CLIP_THRESHOLD {
            clips += 1;
        }
    }
    (peak, clips)
}

pub fn analyze_dynamics(samples: &[f32], frames: &[PitchFrame]) -> DynamicsSummary {
    let (peak, clips) = sample_peak_and_clips(samples);
    let voiced: Vec<&PitchFrame> = frames.iter().filter(|f| f.is_voiced()).collect();
    let levels: Vec<f32> = voiced.iter().map(|f| f.rms_db).collect();
    let times: Vec<f32> = voiced.iter().map(|f| f.time).collect();
    let enough = levels.len() >= 10;
    DynamicsSummary {
        peak_db: gain_to_db(peak),
        voiced_rms_db: crate::stats::median(&levels),
        clipped_samples: clips,
        clip_ratio: if samples.is_empty() {
            0.0
        } else {
            clips as f32 / samples.len() as f32
        },
        dynamic_range_db: if enough {
            Some(percentile(&levels, 95.0).unwrap_or(0.0) - percentile(&levels, 10.0).unwrap_or(0.0))
        } else {
            None
        },
        trend_db_per_second: if enough {
            linear_slope(&times, &levels)
        } else {
            None
        },
    }
}

/// Measure soft-to-strong contrast for a "swell" exercise: compares the mean
/// level of the first and last thirds of the voiced part with the loudest
/// central third. Returns `(start_db, peak_db, end_db)`.
pub fn swell_profile(frames: &[PitchFrame]) -> Option<(f32, f32, f32)> {
    let levels: Vec<f32> = frames.iter().filter(|f| f.is_voiced()).map(|f| f.rms_db).collect();
    if levels.len() < 30 {
        return None;
    }
    let third = levels.len() / 3;
    let avg = |s: &[f32]| s.iter().sum::<f32>() / s.len() as f32;
    let start = avg(&levels[..third]);
    let end = avg(&levels[levels.len() - third..]);
    let mid = percentile(&levels[third..levels.len() - third], 90.0)?;
    Some((start, mid, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::{track_pitch, PitchConfig};
    use crate::synth;

    #[test]
    fn crescendo_has_positive_trend_and_range() {
        let sr = 48_000;
        let mut sig = synth::harmonic_tone(220.0, &[1.0, 0.5], 2.0, sr);
        synth::ramp_gain(&mut sig, 0.1, 1.0); // 20 dB rise
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let d = analyze_dynamics(&sig, &frames);
        assert!(d.trend_db_per_second.unwrap() > 4.0, "{:?}", d);
        let range = d.dynamic_range_db.unwrap();
        assert!(range > 10.0 && range < 22.0, "{range}");
        assert_eq!(d.clipped_samples, 0);
    }

    #[test]
    fn clipping_is_counted() {
        let sig: Vec<f32> = synth::sine(200.0, 1.5, 0.5, 48_000)
            .into_iter()
            .map(|s| s.clamp(-1.0, 1.0))
            .collect();
        let frames = track_pitch(&sig, PitchConfig::new(48_000));
        let d = analyze_dynamics(&sig, &frames);
        assert!(d.clip_ratio > 0.2);
        assert!(d.peak_db > -0.01);
    }
}
