//! Vibrato detection on a pitch contour.

use serde::{Deserialize, Serialize};

use crate::stats::{percentile, linear_slope};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vibrato {
    /// Modulation rate in Hz.
    pub rate_hz: f32,
    /// Peak deviation in cents (half of peak-to-peak).
    pub extent_cents: f32,
    /// Normalised autocorrelation at the vibrato period (0..1): how regular
    /// the oscillation is.
    pub regularity: f32,
}

/// Detect periodic pitch modulation in the 3.5–9 Hz range.
///
/// `cents` must be uniformly sampled every `hop` seconds. Returns `None` if
/// the contour is shorter than ~0.6 s, the modulation is under 12 cents, or
/// the autocorrelation peak is weak (< 0.4).
pub fn detect_vibrato(cents: &[f32], hop: f32) -> Option<Vibrato> {
    if cents.len() as f32 * hop < 0.6 {
        return None;
    }
    // remove linear trend
    let t: Vec<f32> = (0..cents.len()).map(|i| i as f32 * hop).collect();
    let slope = linear_slope(&t, cents).unwrap_or(0.0);
    let m = cents.iter().sum::<f32>() / cents.len() as f32;
    let tm = t.iter().sum::<f32>() / t.len() as f32;
    let x: Vec<f32> = cents
        .iter()
        .zip(&t)
        .map(|(c, ti)| c - m - slope * (ti - tm))
        .collect();

    let energy: f32 = x.iter().map(|v| v * v).sum();
    if energy <= 0.0 {
        return None;
    }
    let lag_min = (1.0 / (9.0 * hop)).floor().max(1.0) as usize;
    let lag_max = ((1.0 / (3.5 * hop)).ceil() as usize).min(x.len() / 2);
    if lag_min >= lag_max {
        return None;
    }
    let mut best_lag = 0;
    let mut best = f32::MIN;
    let mut prev = f32::MIN;
    let mut rising = false;
    for lag in lag_min..=lag_max {
        let r: f32 = x[..x.len() - lag]
            .iter()
            .zip(&x[lag..])
            .map(|(a, b)| a * b)
            .sum::<f32>()
            / energy
            * (x.len() as f32 / (x.len() - lag) as f32);
        // first local maximum after the curve starts rising
        if r > prev {
            rising = true;
        } else if rising && prev > best {
            best = prev;
            best_lag = lag - 1;
            break;
        }
        prev = r;
    }
    if best_lag == 0 || best < 0.4 {
        return None;
    }
    let extent = (percentile(&x, 95.0)? - percentile(&x, 5.0)?) / 2.0;
    if extent < 12.0 {
        return None;
    }
    Some(Vibrato {
        rate_hz: 1.0 / (best_lag as f32 * hop),
        extent_cents: extent,
        regularity: best.min(1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::{track_pitch, PitchConfig};
    use crate::synth;
    use uta_dsp::tone::cents_between;

    #[test]
    fn detects_rate_and_extent() {
        let sr = 48_000;
        let sig = synth::vibrato_tone(220.0, 5.5, 50.0, 2.5, sr);
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let cents: Vec<f32> = frames.iter().filter_map(|f| f.hz.map(|h| cents_between(h, 220.0))).collect();
        let v = detect_vibrato(&cents, 0.01).expect("vibrato");
        assert!((v.rate_hz - 5.5).abs() < 0.5, "{v:?}");
        assert!((v.extent_cents - 48.0).abs() < 8.0, "{v:?}");
    }

    #[test]
    fn no_vibrato_on_steady_or_short_input() {
        let flat = vec![0.0f32; 200];
        assert!(detect_vibrato(&flat, 0.01).is_none());
        let short: Vec<f32> = (0..30).map(|i| 50.0 * (i as f32).sin()).collect();
        assert!(detect_vibrato(&short, 0.01).is_none());
    }
}
