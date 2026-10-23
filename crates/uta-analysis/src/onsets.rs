//! Energy-based syllable onset detection (for rhythm and articulation).
//!
//! Pitch-based note segmentation cannot see repeated syllables on one pitch
//! ("ta ta ta"), so this detector looks for fast rises of a 5 ms RMS envelope
//! instead. It is tuned for close-miked voice; very reverberant rooms blur
//! onsets and are reported as lower-confidence by the recording-quality check.

use serde::{Deserialize, Serialize};

use crate::stats::percentile;
use uta_dsp::level::gain_to_db;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Onset {
    /// Seconds from the start of the signal.
    pub time: f32,
    /// Level reached after the onset (dBFS RMS).
    pub level_db: f32,
}

/// Envelope of RMS levels in dB, one value per `hop` samples.
pub fn rms_envelope_db(samples: &[f32], hop: usize) -> Vec<f32> {
    samples
        .chunks(hop.max(1))
        .map(|c| {
            let e = c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32;
            gain_to_db(e.sqrt())
        })
        .collect()
}

/// Detect onsets: the envelope rises by at least `rise_db` within 40 ms from a
/// local minimum and ends at least 15 dB above the background estimate.
/// Onsets closer than `min_gap` seconds are merged.
pub fn detect_onsets(samples: &[f32], sample_rate: u32, rise_db: f32, min_gap: f32) -> Vec<Onset> {
    let hop = (sample_rate as f32 * 0.005) as usize;
    let env = rms_envelope_db(samples, hop);
    if env.len() < 4 {
        return Vec::new();
    }
    let floor = percentile(&env, 10.0).unwrap_or(-90.0).max(-90.0);
    let lookahead = (0.04 / 0.005) as usize; // 40 ms
    let min_gap_frames = (min_gap / 0.005).round() as usize;
    let mut out: Vec<Onset> = Vec::new();
    let mut i = 1;
    while i + 1 < env.len() {
        // local minimum (or start of a rise)
        if env[i] <= env[i - 1] && env[i] < env[i + 1] {
            let end = (i + lookahead).min(env.len() - 1);
            let (peak_idx, peak) = env[i..=end]
                .iter()
                .enumerate()
                .fold((0, f32::MIN), |(bi, bv), (k, v)| if *v > bv { (k, *v) } else { (bi, bv) });
            if peak - env[i] >= rise_db && peak >= floor + 15.0 {
                // onset time = first frame that crossed halfway up the rise
                let half = env[i] + (peak - env[i]) / 2.0;
                let k = (i..=i + peak_idx).find(|&k| env[k] >= half).unwrap_or(i);
                let t = k as f32 * 0.005;
                match out.last_mut() {
                    Some(last) if ((t - last.time) / 0.005) < min_gap_frames as f32 => {
                        if peak > last.level_db {
                            last.level_db = peak;
                        }
                    }
                    _ => out.push(Onset { time: t, level_db: peak }),
                }
                i += peak_idx.max(1);
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Match detected onsets to expected times (seconds). Returns, per expected
/// time, the signed error in ms of the closest onset within `window` seconds.
pub fn match_onsets(onsets: &[Onset], expected: &[f32], window: f32) -> Vec<Option<f32>> {
    let mut used = vec![false; onsets.len()];
    expected
        .iter()
        .map(|&e| {
            let best = onsets
                .iter()
                .enumerate()
                .filter(|(k, o)| !used[*k] && (o.time - e).abs() <= window)
                .min_by(|a, b| (a.1.time - e).abs().total_cmp(&(b.1.time - e).abs()));
            best.map(|(k, o)| {
                used[k] = true;
                (o.time - e) * 1000.0
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;

    /// "ta ta ta": 120 ms bursts every 500 ms with a 5 ms attack.
    fn syllables(times: &[f32], sr: u32, total: f32) -> Vec<f32> {
        let mut out = synth::noise(0.0005, total, sr, 9);
        let tone = synth::harmonic_tone(150.0, &[1.0, 0.5, 0.3], 0.12, sr);
        for &t in times {
            let s = (t * sr as f32) as usize;
            for (k, v) in tone.iter().enumerate() {
                let att = (k as f32 / (0.005 * sr as f32)).min(1.0);
                if s + k < out.len() {
                    out[s + k] += v * 0.5 * att;
                }
            }
        }
        out
    }

    #[test]
    fn finds_syllable_onsets() {
        let sr = 48_000;
        let times = [0.3, 0.8, 1.3, 1.8, 2.3];
        let sig = syllables(&times, sr, 2.8);
        let on = detect_onsets(&sig, sr, 12.0, 0.08);
        assert_eq!(on.len(), times.len(), "{on:?}");
        for (o, t) in on.iter().zip(times) {
            assert!((o.time - t).abs() < 0.012, "{} vs {t}", o.time);
        }
        let m = match_onsets(&on, &[0.3, 0.8, 1.35, 5.0], 0.15);
        assert!(m[0].unwrap().abs() < 12.0);
        assert!((m[2].unwrap() + 50.0).abs() < 12.0);
        assert!(m[3].is_none());
    }

    #[test]
    fn steady_tone_and_silence_have_no_onsets_after_start() {
        let sr = 48_000;
        let sig = vec![0.0f32; sr as usize];
        assert!(detect_onsets(&sig, sr, 12.0, 0.08).is_empty());
        let mut tone = vec![0.0f32; sr as usize / 4];
        tone.extend(synth::harmonic_tone(200.0, &[1.0, 0.4], 1.5, sr).iter().map(|s| s * 0.4));
        let on = detect_onsets(&tone, sr, 12.0, 0.08);
        assert_eq!(on.len(), 1, "{on:?}");
    }
}
