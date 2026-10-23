//! Spectral features: brightness (centroid), high-frequency energy ratio and
//! low-harmonic structure.
//!
//! These are *acoustic* descriptions of the recorded sound. They are strongly
//! affected by the microphone, its distance and angle, the room and any EQ, so
//! they are only compared between takes made with the same setup, and never
//! interpreted as statements about the singer's anatomy or vocal health.

use std::sync::Arc;

use realfft::{num_complex::Complex, RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

use crate::pitch::PitchFrame;
use crate::stats::{median, percentile};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectralFrame {
    pub time: f32,
    /// Spectral centroid in Hz ("brightness").
    pub centroid_hz: f32,
    /// Energy above 3 kHz relative to total, in dB (≤ 0).
    pub high_ratio_db: f32,
    /// Level of the 2nd harmonic relative to the 1st (H1−H2, dB). Only for
    /// voiced frames with a reliable f0.
    pub h1_h2_db: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectralSummary {
    pub median_centroid_hz: Option<f32>,
    /// Spread of brightness across the take (p90 − p10 of centroid, Hz):
    /// a rough indicator of timbre change.
    pub centroid_spread_hz: Option<f32>,
    pub median_high_ratio_db: Option<f32>,
    pub median_h1_h2_db: Option<f32>,
    /// Mean YIN periodicity of voiced frames (0..1). Lower values *can* go
    /// with a breathier sound, but also with noise, reverb or vibrato, so this
    /// is reported as "periodicity", not as a breathiness diagnosis.
    pub mean_periodicity: Option<f32>,
}

pub struct SpectralAnalyzer {
    sr: f32,
    size: usize,
    window: Vec<f32>,
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    mags: Vec<f32>,
}

impl SpectralAnalyzer {
    pub fn new(sample_rate: u32) -> Self {
        let size = if sample_rate > 50_000 { 4096 } else { 2048 };
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(size);
        let window = (0..size)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / size as f32).cos())
            .collect();
        Self {
            sr: sample_rate as f32,
            size,
            window,
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            mags: vec![0.0; size / 2 + 1],
            fft,
        }
    }

    /// Analyse the window centred at `center` samples.
    pub fn frame(&mut self, samples: &[f32], center: usize, f0: Option<f32>, time: f32) -> SpectralFrame {
        let half = self.size / 2;
        for i in 0..self.size {
            let idx = center as isize - half as isize + i as isize;
            let s = if idx >= 0 && (idx as usize) < samples.len() {
                samples[idx as usize]
            } else {
                0.0
            };
            self.input[i] = s * self.window[i];
        }
        self.fft
            .process(&mut self.input, &mut self.spectrum)
            .expect("fft buffer sizes");
        let bin_hz = self.sr / self.size as f32;
        let mut total = 0.0f64;
        let mut weighted = 0.0f64;
        let mut high = 0.0f64;
        for (k, c) in self.spectrum.iter().enumerate() {
            let p = c.norm_sqr() as f64;
            self.mags[k] = p.sqrt() as f32;
            let f = k as f64 * bin_hz as f64;
            if f < 60.0 {
                continue; // ignore rumble / DC
            }
            total += p;
            weighted += p * f;
            if f >= 3000.0 {
                high += p;
            }
        }
        let centroid = if total > 0.0 { (weighted / total) as f32 } else { 0.0 };
        let high_ratio_db = if total > 0.0 && high > 0.0 {
            (10.0 * (high / total).log10()) as f32
        } else {
            -120.0
        };
        let h1_h2_db = f0.and_then(|f0| {
            let h1 = self.harmonic_peak(f0, bin_hz)?;
            let h2 = self.harmonic_peak(2.0 * f0, bin_hz)?;
            Some(20.0 * (h1 / h2).log10())
        });
        SpectralFrame {
            time,
            centroid_hz: centroid,
            high_ratio_db,
            h1_h2_db,
        }
    }

    fn harmonic_peak(&self, hz: f32, bin_hz: f32) -> Option<f32> {
        let k = (hz / bin_hz).round() as usize;
        if k < 1 || k + 2 >= self.mags.len() {
            return None;
        }
        let peak = self.mags[k - 1..=k + 1].iter().fold(0.0f32, |m, v| m.max(*v));
        if peak > 1e-6 {
            Some(peak)
        } else {
            None
        }
    }
}

/// Spectral analysis of voiced frames, decimated to at most ~50 frames/s.
pub fn analyze_spectrum(samples: &[f32], sample_rate: u32, frames: &[PitchFrame]) -> (Vec<SpectralFrame>, SpectralSummary) {
    let mut an = SpectralAnalyzer::new(sample_rate);
    let step = if frames.len() > 1 {
        let hop = frames[1].time - frames[0].time;
        ((0.02 / hop.max(1e-4)).round() as usize).max(1)
    } else {
        1
    };
    let mut out = Vec::new();
    for f in frames.iter().step_by(step).filter(|f| f.is_voiced()) {
        let center = (f.time * sample_rate as f32) as usize;
        out.push(an.frame(samples, center, f.hz, f.time));
    }
    let centroids: Vec<f32> = out.iter().map(|f| f.centroid_hz).collect();
    let highs: Vec<f32> = out.iter().map(|f| f.high_ratio_db).collect();
    let h12: Vec<f32> = out.iter().filter_map(|f| f.h1_h2_db).collect();
    let conf: Vec<f32> = frames.iter().filter(|f| f.is_voiced()).map(|f| f.confidence).collect();
    let summary = SpectralSummary {
        median_centroid_hz: median(&centroids),
        centroid_spread_hz: if centroids.len() >= 5 {
            Some(percentile(&centroids, 90.0).unwrap_or(0.0) - percentile(&centroids, 10.0).unwrap_or(0.0))
        } else {
            None
        },
        median_high_ratio_db: median(&highs),
        median_h1_h2_db: median(&h12),
        mean_periodicity: crate::stats::mean(&conf),
    };
    (out, summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::{track_pitch, PitchConfig};
    use crate::synth;

    #[test]
    fn brighter_tone_has_higher_centroid() {
        let sr = 48_000;
        let dark = synth::harmonic_tone(220.0, &[1.0, 0.3, 0.1], 1.0, sr);
        let bright = synth::harmonic_tone(220.0, &[1.0, 0.8, 0.8, 0.7, 0.7, 0.6, 0.6, 0.5, 0.5, 0.5], 1.0, sr);
        let f1 = track_pitch(&dark, PitchConfig::new(sr));
        let f2 = track_pitch(&bright, PitchConfig::new(sr));
        let (_, s1) = analyze_spectrum(&dark, sr, &f1);
        let (_, s2) = analyze_spectrum(&bright, sr, &f2);
        let c1 = s1.median_centroid_hz.unwrap();
        let c2 = s2.median_centroid_hz.unwrap();
        assert!(c2 > c1 * 1.5, "{c1} {c2}");
        // H1-H2 of the dark tone: 20log10(1/0.3) ≈ 10.5 dB
        let h = s1.median_h1_h2_db.unwrap();
        assert!((h - 10.46).abs() < 1.5, "{h}");
    }
}
