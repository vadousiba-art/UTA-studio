//! Deterministic synthetic test signals. Used by unit tests, integration
//! tests and the audio engine's fixture tests — never shipped as user content.

use std::f32::consts::TAU;

pub fn sine(hz: f32, amp: f32, seconds: f32, sr: u32) -> Vec<f32> {
    let n = (seconds * sr as f32) as usize;
    (0..n)
        .map(|i| amp * (TAU * hz * i as f32 / sr as f32).sin())
        .collect()
}

/// Sum of harmonics with given relative amplitudes (index 0 = fundamental),
/// normalised to a 0.8 peak.
pub fn harmonic_tone(hz: f32, harmonics: &[f32], seconds: f32, sr: u32) -> Vec<f32> {
    let n = (seconds * sr as f32) as usize;
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / sr as f32;
            harmonics
                .iter()
                .enumerate()
                .map(|(k, a)| a * (TAU * hz * (k + 1) as f32 * t).sin())
                .sum()
        })
        .collect();
    normalise(&mut out, 0.8);
    out
}

/// Exponential glide from `f0` to `f1` Hz.
pub fn glide(f0: f32, f1: f32, seconds: f32, amp: f32, sr: u32) -> Vec<f32> {
    let n = (seconds * sr as f32) as usize;
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let f = f0 * (f1 / f0).powf(t);
            phase += f / sr as f32;
            if phase > 1.0 {
                phase -= 1.0;
            }
            amp * ((TAU * phase).sin() + 0.4 * (2.0 * TAU * phase).sin()) / 1.4
        })
        .collect()
}

/// Tone with sinusoidal vibrato of `depth_cents` (peak) at `rate_hz`.
pub fn vibrato_tone(hz: f32, rate_hz: f32, depth_cents: f32, seconds: f32, sr: u32) -> Vec<f32> {
    let n = (seconds * sr as f32) as usize;
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / sr as f32;
            let cents = depth_cents * (TAU * rate_hz * t).sin();
            let f = hz * 2f32.powf(cents / 1200.0);
            phase += f / sr as f32;
            if phase > 1.0 {
                phase -= 1.0;
            }
            0.5 * ((TAU * phase).sin() + 0.3 * (2.0 * TAU * phase).sin())
        })
        .collect()
}

/// Tone whose pitch drifts linearly by `drift_cents` over its duration.
pub fn drifting_tone(hz: f32, drift_cents: f32, seconds: f32, sr: u32) -> Vec<f32> {
    let n = (seconds * sr as f32) as usize;
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let cents = drift_cents * i as f32 / n as f32;
            let f = hz * 2f32.powf(cents / 1200.0);
            phase += f / sr as f32;
            if phase > 1.0 {
                phase -= 1.0;
            }
            0.5 * ((TAU * phase).sin() + 0.3 * (2.0 * TAU * phase).sin())
        })
        .collect()
}

/// Uniform white noise from a small LCG (deterministic for a given seed).
pub fn noise(amp: f32, seconds: f32, sr: u32, seed: u32) -> Vec<f32> {
    let n = (seconds * sr as f32) as usize;
    let mut s = seed.wrapping_mul(2654435761).wrapping_add(1);
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            amp * ((s >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0)
        })
        .collect()
}

/// Sequence of notes `(midi, seconds)` with short gaps and 10 ms fades,
/// optionally starting after `lead_in` seconds of silence.
pub fn note_sequence(notes: &[(f32, f32)], gap: f32, lead_in: f32, sr: u32) -> Vec<f32> {
    let mut out = vec![0.0; (lead_in * sr as f32) as usize];
    let fade = (0.01 * sr as f32) as usize;
    for &(midi, dur) in notes {
        let hz = uta_dsp::tone::midi_to_hz(midi);
        let mut tone = harmonic_tone(hz, &[1.0, 0.5, 0.25], dur, sr);
        let len = tone.len();
        for i in 0..fade.min(len / 2) {
            let g = i as f32 / fade as f32;
            tone[i] *= g;
            tone[len - 1 - i] *= g;
        }
        out.extend(tone.iter().map(|s| s * 0.6));
        out.extend(std::iter::repeat_n(0.0, (gap * sr as f32) as usize));
    }
    out
}

pub fn normalise(sig: &mut [f32], peak: f32) {
    let m = sig.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if m > 0.0 {
        let g = peak / m;
        sig.iter_mut().for_each(|s| *s *= g);
    }
}

/// Apply a linear gain ramp from `g0` to `g1` across the signal.
pub fn ramp_gain(sig: &mut [f32], g0: f32, g1: f32) {
    let n = sig.len().max(1) as f32;
    for (i, s) in sig.iter_mut().enumerate() {
        *s *= g0 + (g1 - g0) * i as f32 / n;
    }
}
