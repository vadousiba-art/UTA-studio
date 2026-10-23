//! Peak / RMS metering with clip detection.
//!
//! The meter accumulates statistics on the audio thread; a reader (UI bridge
//! thread) periodically calls [`PeakRmsMeter::take`] through whatever
//! lock-free publication mechanism the caller uses (the engine copies readings
//! into atomics).

use crate::level::gain_to_db;

/// A snapshot of meter state since the previous read.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeterReading {
    /// Highest absolute sample value since last read (linear).
    pub peak: f32,
    /// RMS over the samples since last read (linear).
    pub rms: f32,
    /// Number of samples at or above the clip threshold.
    pub clipped_samples: u32,
}

impl MeterReading {
    pub fn peak_db(&self) -> f32 {
        gain_to_db(self.peak)
    }
    pub fn rms_db(&self) -> f32 {
        gain_to_db(self.rms)
    }
}

/// Samples whose magnitude is at or above this are counted as clipped.
/// Slightly below full scale because many converters clip just short of 1.0.
pub const CLIP_THRESHOLD: f32 = 0.999;

#[derive(Debug, Clone, Default)]
pub struct PeakRmsMeter {
    peak: f32,
    sum_sq: f64,
    count: u64,
    clipped: u32,
}

impl PeakRmsMeter {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn process(&mut self, samples: &[f32]) {
        let mut peak = self.peak;
        let mut sum = 0.0f64;
        let mut clipped = 0u32;
        for &s in samples {
            let a = s.abs();
            if a > peak {
                peak = a;
            }
            if a >= CLIP_THRESHOLD {
                clipped += 1;
            }
            sum += (s as f64) * (s as f64);
        }
        self.peak = peak;
        self.sum_sq += sum;
        self.count += samples.len() as u64;
        self.clipped = self.clipped.saturating_add(clipped);
    }

    /// Read and reset the accumulated values.
    pub fn take(&mut self) -> MeterReading {
        let rms = if self.count > 0 {
            (self.sum_sq / self.count as f64).sqrt() as f32
        } else {
            0.0
        };
        let r = MeterReading {
            peak: self.peak,
            rms,
            clipped_samples: self.clipped,
        };
        *self = Self::default();
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_rms_and_peak() {
        let mut m = PeakRmsMeter::new();
        let sr = 48_000.0;
        let buf: Vec<f32> = (0..48_000)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr).sin())
            .collect();
        m.process(&buf);
        let r = m.take();
        assert!((r.peak - 0.5).abs() < 1e-3);
        assert!((r.rms - 0.5 / 2f32.sqrt()).abs() < 1e-3);
        assert_eq!(r.clipped_samples, 0);
        // reset after take
        assert_eq!(m.take().peak, 0.0);
    }

    #[test]
    fn detects_clipping() {
        let mut m = PeakRmsMeter::new();
        m.process(&[0.1, 1.0, -1.0, 0.2, 1.3]);
        let r = m.take();
        assert_eq!(r.clipped_samples, 3);
        assert!((r.peak - 1.3).abs() < 1e-6);
    }
}
