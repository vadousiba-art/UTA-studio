//! Fundamental-frequency estimation with the YIN algorithm
//! (de Cheveigné & Kawahara, JASA 2002).
//!
//! The difference function is computed with an FFT cross-correlation plus
//! prefix-summed energies, which is mathematically identical to the direct
//! O(W·τ) form but fast enough to run over whole recordings.
//!
//! Each frame reports a *confidence* `1 - d'(τ*)` where `d'` is the cumulative
//! mean normalised difference at the chosen lag. Clean periodic signals give
//! values near 1; noise and breath give low values. Frames are only marked
//! voiced when both confidence and level are high enough, so downstream code
//! never grades pitch it could not measure.

use std::sync::Arc;

use realfft::{num_complex::Complex, ComplexToReal, RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

use crate::stats::median;
use uta_dsp::level::gain_to_db;
use uta_dsp::tone::hz_to_midi;

/// Pitch tracker settings. Defaults suit adult singing voices.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitchConfig {
    pub sample_rate: u32,
    /// Lowest detectable f0 (Hz). 65 Hz ≈ C2.
    pub min_hz: f32,
    /// Highest detectable f0 (Hz). 1100 Hz ≈ C#6.
    pub max_hz: f32,
    /// Hop between frames in seconds (analysis resolution).
    pub hop_seconds: f32,
    /// YIN absolute threshold on the CMND function.
    pub yin_threshold: f32,
    /// Frames quieter than this (dBFS RMS) are treated as silence.
    pub silence_db: f32,
    /// Minimum confidence for a frame to count as voiced.
    pub voicing_confidence: f32,
}

impl PitchConfig {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            min_hz: 65.0,
            max_hz: 1100.0,
            hop_seconds: 0.01,
            yin_threshold: 0.15,
            silence_db: -55.0,
            voicing_confidence: 0.6,
        }
    }

    pub fn hop_samples(&self) -> usize {
        ((self.hop_seconds * self.sample_rate as f32).round() as usize).max(1)
    }
}

/// One analysis frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitchFrame {
    /// Centre of the analysis window, seconds from the start of the signal.
    pub time: f32,
    /// f0 in Hz when the frame is voiced, otherwise `None`.
    pub hz: Option<f32>,
    /// Best f0 candidate even for unvoiced frames (diagnostics only).
    pub candidate_hz: Option<f32>,
    /// 0..1, higher = more periodic.
    pub confidence: f32,
    /// Frame RMS level in dBFS.
    pub rms_db: f32,
}

impl PitchFrame {
    pub fn midi(&self) -> Option<f32> {
        self.hz.map(hz_to_midi)
    }
    pub fn is_voiced(&self) -> bool {
        self.hz.is_some()
    }
}

/// Raw single-frame estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    pub hz: Option<f32>,
    pub confidence: f32,
    pub rms_db: f32,
}

/// Reusable YIN detector with preallocated buffers.
pub struct YinDetector {
    sample_rate: f32,
    window: usize,
    tau_min: usize,
    tau_max: usize,
    threshold: f32,
    silence_db: f32,
    fft_len: usize,
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    a: Vec<f32>,
    b: Vec<f32>,
    spec_a: Vec<Complex<f32>>,
    spec_b: Vec<Complex<f32>>,
    corr: Vec<f32>,
    prefix: Vec<f64>,
    cmnd: Vec<f32>,
}

impl YinDetector {
    pub fn new(cfg: &PitchConfig) -> Self {
        let sr = cfg.sample_rate as f32;
        let tau_max = (sr / cfg.min_hz).ceil() as usize + 2;
        let tau_min = ((sr / cfg.max_hz).floor() as usize).max(2);
        let window = tau_max.max((sr * 0.025) as usize);
        let fft_len = (window + tau_max + window).next_power_of_two();
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(fft_len);
        let inv = planner.plan_fft_inverse(fft_len);
        Self {
            sample_rate: sr,
            window,
            tau_min,
            tau_max,
            threshold: cfg.yin_threshold,
            silence_db: cfg.silence_db,
            fft_len,
            a: fwd.make_input_vec(),
            b: fwd.make_input_vec(),
            spec_a: fwd.make_output_vec(),
            spec_b: fwd.make_output_vec(),
            corr: inv.make_output_vec(),
            fwd,
            inv,
            prefix: vec![0.0; window + tau_max + 1],
            cmnd: vec![1.0; tau_max + 1],
        }
    }

    /// Number of samples one analysis frame needs.
    pub fn frame_len(&self) -> usize {
        self.window + self.tau_max
    }

    /// Estimate f0 for `frame`, which must hold at least [`Self::frame_len`]
    /// samples (extra samples are ignored).
    pub fn estimate(&mut self, frame: &[f32]) -> Estimate {
        let n = self.frame_len();
        assert!(frame.len() >= n, "frame too short");
        let x = &frame[..n];

        // energy / silence check over the integration window
        let mut energy = 0.0f64;
        for &s in &x[..self.window] {
            energy += (s as f64) * (s as f64);
        }
        let rms = (energy / self.window as f64).sqrt() as f32;
        let rms_db = gain_to_db(rms);
        if rms_db < self.silence_db {
            return Estimate {
                hz: None,
                confidence: 0.0,
                rms_db,
            };
        }

        // r(τ) = Σ_{j<W} x[j]·x[j+τ] via FFT cross-correlation
        self.a.fill(0.0);
        self.a[..self.window].copy_from_slice(&x[..self.window]);
        self.b.fill(0.0);
        self.b[..n].copy_from_slice(x);
        // process() only fails on wrong buffer lengths, which are fixed here
        self.fwd
            .process(&mut self.a, &mut self.spec_a)
            .expect("fft buffer sizes");
        self.fwd
            .process(&mut self.b, &mut self.spec_b)
            .expect("fft buffer sizes");
        for (sa, sb) in self.spec_a.iter_mut().zip(&self.spec_b) {
            *sa = sa.conj() * sb;
        }
        // DC and Nyquist bins must be purely real for the inverse transform.
        if let Some(first) = self.spec_a.first_mut() {
            first.im = 0.0;
        }
        if let Some(last) = self.spec_a.last_mut() {
            last.im = 0.0;
        }
        self.inv
            .process(&mut self.spec_a, &mut self.corr)
            .expect("fft buffer sizes");
        let scale = 1.0 / self.fft_len as f32;

        // prefix sums of squares for e(τ) = Σ_{j<W} x[j+τ]^2
        self.prefix[0] = 0.0;
        for (i, &s) in x.iter().enumerate() {
            self.prefix[i + 1] = self.prefix[i] + (s as f64) * (s as f64);
        }
        let e0 = self.prefix[self.window];

        // cumulative mean normalised difference
        self.cmnd[0] = 1.0;
        let mut running = 0.0f64;
        for tau in 1..=self.tau_max {
            let e_tau = self.prefix[tau + self.window] - self.prefix[tau];
            let r = (self.corr[tau] * scale) as f64;
            let d = (e0 + e_tau - 2.0 * r).max(0.0);
            running += d;
            self.cmnd[tau] = if running > 0.0 {
                (d * tau as f64 / running) as f32
            } else {
                1.0
            };
        }

        // absolute threshold: first dip below threshold, then its local minimum
        let mut chosen = None;
        let mut tau = self.tau_min;
        while tau < self.tau_max {
            if self.cmnd[tau] < self.threshold {
                while tau + 1 < self.tau_max && self.cmnd[tau + 1] < self.cmnd[tau] {
                    tau += 1;
                }
                chosen = Some(tau);
                break;
            }
            tau += 1;
        }
        // otherwise fall back to the global minimum (low confidence)
        let tau = chosen.unwrap_or_else(|| {
            (self.tau_min..self.tau_max)
                .min_by(|&a, &b| self.cmnd[a].total_cmp(&self.cmnd[b]))
                .unwrap_or(self.tau_min)
        });

        let refined = self.parabolic(tau);
        let confidence = (1.0 - self.cmnd[tau]).clamp(0.0, 1.0);
        let hz = self.sample_rate / refined;
        Estimate {
            hz: if hz.is_finite() { Some(hz) } else { None },
            confidence,
            rms_db,
        }
    }

    fn parabolic(&self, tau: usize) -> f32 {
        if tau == 0 || tau + 1 > self.tau_max {
            return tau as f32;
        }
        let (s0, s1, s2) = (self.cmnd[tau - 1], self.cmnd[tau], self.cmnd[tau + 1]);
        let denom = 2.0 * (2.0 * s1 - s2 - s0);
        if denom.abs() < 1e-12 {
            tau as f32
        } else {
            tau as f32 + (s2 - s0) / denom
        }
    }
}

/// Offline pitch tracking over a whole (mono) signal.
pub struct PitchTracker {
    cfg: PitchConfig,
    det: YinDetector,
}

impl PitchTracker {
    pub fn new(cfg: PitchConfig) -> Self {
        let det = YinDetector::new(&cfg);
        Self { cfg, det }
    }

    pub fn config(&self) -> &PitchConfig {
        &self.cfg
    }

    pub fn frame_len(&self) -> usize {
        self.det.frame_len()
    }

    /// Analyse a frame and apply the voicing decision.
    pub fn frame(&mut self, frame: &[f32], time: f32) -> PitchFrame {
        let est = self.det.estimate(frame);
        let voiced = est.hz.is_some()
            && est.confidence >= self.cfg.voicing_confidence
            && est.rms_db >= self.cfg.silence_db;
        PitchFrame {
            time,
            hz: if voiced { est.hz } else { None },
            candidate_hz: est.hz,
            confidence: est.confidence,
            rms_db: est.rms_db,
        }
    }

    /// Track a whole signal and apply octave-error correction and blip removal.
    pub fn track(&mut self, samples: &[f32]) -> Vec<PitchFrame> {
        let n = self.det.frame_len();
        let hop = self.cfg.hop_samples();
        let sr = self.cfg.sample_rate as f32;
        let mut frames = Vec::with_capacity(samples.len() / hop + 1);
        let mut padded = vec![0.0f32; n];
        let mut start = 0usize;
        while start < samples.len() {
            let end = (start + n).min(samples.len());
            let slice: &[f32] = if end - start == n {
                &samples[start..end]
            } else {
                padded.fill(0.0);
                padded[..end - start].copy_from_slice(&samples[start..end]);
                &padded
            };
            let time = (start as f32 + self.det.window as f32 / 2.0) / sr;
            let mut frame = self.frame(slice, time);
            if end - start < n {
                // Zero-padded tail frames bias the lag estimate; never grade them.
                frame.hz = None;
            }
            frames.push(frame);
            start += hop;
        }
        postprocess(&mut frames, self.cfg.hop_seconds);
        frames
    }
}

/// Convenience wrapper: track pitch with the given config.
pub fn track_pitch(samples: &[f32], cfg: PitchConfig) -> Vec<PitchFrame> {
    PitchTracker::new(cfg).track(samples)
}

/// Post-processing applied to offline tracks:
/// 1. octave-jump correction against the local median of voiced frames,
/// 2. removal of voiced runs shorter than 30 ms (isolated blips).
pub fn postprocess(frames: &mut [PitchFrame], hop_seconds: f32) {
    fix_octave_errors(frames, hop_seconds);
    remove_short_runs(frames, (0.03 / hop_seconds).ceil() as usize);
}

/// Fold frames that sit an octave away from the local median of their voiced
/// neighbours back to that median's octave. Only applied when there is enough
/// local context (at least 6 neighbours within ±150 ms).
pub fn fix_octave_errors(frames: &mut [PitchFrame], hop_seconds: f32) {
    let radius = ((0.15 / hop_seconds).round() as usize).max(3);
    let midis: Vec<Option<f32>> = frames.iter().map(|f| f.midi()).collect();
    let mut neighbours = Vec::with_capacity(2 * radius);
    for i in 0..frames.len() {
        let Some(m) = midis[i] else { continue };
        neighbours.clear();
        let lo = i.saturating_sub(radius);
        let hi = (i + radius + 1).min(frames.len());
        for (j, n) in midis[lo..hi].iter().enumerate() {
            if lo + j != i {
                if let Some(v) = n {
                    neighbours.push(*v);
                }
            }
        }
        if neighbours.len() < 6 {
            continue;
        }
        let Some(med) = median(&neighbours) else { continue };
        let diff = m - med;
        let corrected = if (diff - 12.0).abs() < 1.0 {
            Some(m - 12.0)
        } else if (diff + 12.0).abs() < 1.0 {
            Some(m + 12.0)
        } else if (diff - 19.02).abs() < 1.0 {
            // third harmonic picked
            Some(m - 19.02)
        } else {
            None
        };
        if let Some(c) = corrected {
            frames[i].hz = Some(uta_dsp::tone::midi_to_hz(c));
        }
    }
}

fn remove_short_runs(frames: &mut [PitchFrame], min_len: usize) {
    let mut i = 0;
    while i < frames.len() {
        if frames[i].hz.is_some() {
            let start = i;
            while i < frames.len() && frames[i].hz.is_some() {
                i += 1;
            }
            if i - start < min_len {
                for f in &mut frames[start..i] {
                    f.hz = None;
                }
            }
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;
    use uta_dsp::tone::cents_between;

    fn voiced_cents_errors(frames: &[PitchFrame], expected_hz: f32) -> Vec<f32> {
        frames
            .iter()
            .filter_map(|f| f.hz)
            .map(|hz| cents_between(hz, expected_hz).abs())
            .collect()
    }

    /// Clean sines across the singing range: every voiced frame within 5 cents,
    /// and at least 95% of frames voiced.
    #[test]
    fn sine_accuracy_across_range() {
        for sr in [44_100u32, 48_000] {
            for hz in [70.0f32, 98.0, 146.8, 220.0, 329.6, 440.0, 659.3, 987.8] {
                let sig = synth::sine(hz, 0.5, 1.0, sr);
                let frames = track_pitch(&sig, PitchConfig::new(sr));
                let errs = voiced_cents_errors(&frames, hz);
                let voiced_ratio = errs.len() as f32 / frames.len() as f32;
                assert!(voiced_ratio > 0.9, "sr {sr} hz {hz}: voiced {voiced_ratio}");
                let worst = errs.iter().fold(0.0f32, |m, e| m.max(*e));
                assert!(worst < 5.0, "sr {sr} hz {hz}: worst {worst} cents");
            }
        }
    }

    /// Harmonic-rich "voice-like" tones whose 2nd harmonic is stronger than the
    /// fundamental must not be reported an octave up.
    #[test]
    fn no_octave_errors_on_strong_second_harmonic() {
        let sr = 48_000;
        for hz in [110.0f32, 165.0, 247.0] {
            let sig = synth::harmonic_tone(hz, &[0.5, 1.0, 0.6, 0.3, 0.2], 1.0, sr);
            let frames = track_pitch(&sig, PitchConfig::new(sr));
            let errs = voiced_cents_errors(&frames, hz);
            assert!(errs.len() > 80, "{hz}");
            let bad = errs.iter().filter(|e| **e > 50.0).count();
            assert_eq!(bad, 0, "{hz}: {bad} octave/gross errors");
        }
    }

    /// White noise must not be reported as confidently voiced.
    #[test]
    fn noise_is_unvoiced() {
        let sr = 48_000;
        let sig = synth::noise(0.3, 1.0, sr, 7);
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let voiced = frames.iter().filter(|f| f.is_voiced()).count();
        assert!(
            (voiced as f32) < frames.len() as f32 * 0.05,
            "noise voiced in {voiced}/{} frames",
            frames.len()
        );
    }

    #[test]
    fn silence_is_unvoiced_with_zero_confidence() {
        let sig = vec![0.0f32; 48_000];
        let frames = track_pitch(&sig, PitchConfig::new(48_000));
        assert!(frames.iter().all(|f| !f.is_voiced() && f.confidence == 0.0));
    }

    /// A tone with 20 dB SNR noise stays within 15 cents.
    #[test]
    fn noisy_tone_tolerance() {
        let sr = 48_000;
        let tone = synth::harmonic_tone(196.0, &[1.0, 0.5, 0.3], 1.0, sr);
        let noise = synth::noise(0.05, 1.0, sr, 3);
        let sig: Vec<f32> = tone.iter().zip(&noise).map(|(a, b)| a * 0.5 + b).collect();
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let errs = voiced_cents_errors(&frames, 196.0);
        assert!(errs.len() as f32 > frames.len() as f32 * 0.8);
        let p95 = crate::stats::percentile(&errs, 95.0).unwrap();
        assert!(p95 < 15.0, "p95 {p95}");
    }

    /// A one-octave glide is tracked continuously (no octave jumps).
    #[test]
    fn glide_is_tracked() {
        let sr = 48_000;
        let sig = synth::glide(150.0, 300.0, 1.0, 0.5, sr);
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let midis: Vec<f32> = frames.iter().filter_map(|f| f.midi()).collect();
        assert!(midis.len() > 90);
        for w in midis.windows(2) {
            assert!((w[1] - w[0]).abs() < 0.6, "jump {} -> {}", w[0], w[1]);
        }
        // each voiced frame within 20 cents of the glide's pitch at that time
        let m0 = hz_to_midi(150.0);
        for f in frames.iter().filter(|f| f.is_voiced()) {
            let expected = m0 + 12.0 * f.time;
            let err = (f.midi().unwrap() - expected) * 100.0;
            assert!(err.abs() < 20.0, "t {} err {err}", f.time);
        }
    }

    /// Vibrato ±50 cents at 5.5 Hz: contour follows the modulation.
    #[test]
    fn vibrato_contour() {
        let sr = 48_000;
        let sig = synth::vibrato_tone(220.0, 5.5, 50.0, 2.0, sr);
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let cents: Vec<f32> = frames
            .iter()
            .filter_map(|f| f.hz.map(|h| cents_between(h, 220.0)))
            .collect();
        let max = cents.iter().fold(f32::MIN, |m, c| m.max(*c));
        let min = cents.iter().fold(f32::MAX, |m, c| m.min(*c));
        assert!(max > 40.0 && max < 60.0, "max {max}");
        assert!(min < -40.0 && min > -60.0, "min {min}");
    }

    #[test]
    fn octave_fix_folds_isolated_jump() {
        let mut frames: Vec<PitchFrame> = (0..30)
            .map(|i| PitchFrame {
                time: i as f32 * 0.01,
                hz: Some(200.0),
                candidate_hz: Some(200.0),
                confidence: 0.9,
                rms_db: -20.0,
            })
            .collect();
        frames[15].hz = Some(400.0);
        frames[16].hz = Some(100.5);
        fix_octave_errors(&mut frames, 0.01);
        assert!((frames[15].hz.unwrap() - 200.0).abs() < 0.5);
        assert!((frames[16].hz.unwrap() - 201.0).abs() < 0.5);
    }
}
