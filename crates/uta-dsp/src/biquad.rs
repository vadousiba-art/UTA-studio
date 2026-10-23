//! Biquad filters (RBJ Audio EQ Cookbook), transposed direct form II with
//! `f64` state for good low-frequency behaviour.

use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterShape {
    LowPass,
    HighPass,
    Peak,
    LowShelf,
    HighShelf,
}

/// Normalised biquad coefficients (a0 == 1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coeffs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coeffs {
    pub const IDENTITY: Coeffs = Coeffs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Design a filter. `freq` is clamped below Nyquist, `q` to a sane range.
    pub fn design(shape: FilterShape, sample_rate: f32, freq: f32, q: f32, gain_db: f32) -> Coeffs {
        let fs = sample_rate as f64;
        let f = (freq as f64).clamp(10.0, fs * 0.49);
        let q = (q as f64).clamp(0.1, 24.0);
        let w0 = 2.0 * PI * f / fs;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a = 10f64.powf(gain_db as f64 / 40.0);

        let (b0, b1, b2, a0, a1, a2) = match shape {
            FilterShape::LowPass => {
                let b1 = 1.0 - cos;
                (b1 / 2.0, b1, b1 / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            FilterShape::HighPass => {
                let b1 = -(1.0 + cos);
                (-b1 / 2.0, b1, -b1 / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            FilterShape::Peak => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            FilterShape::LowShelf => {
                let sq = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + sq),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - sq),
                    (a + 1.0) + (a - 1.0) * cos + sq,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - sq,
                )
            }
            FilterShape::HighShelf => {
                let sq = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + sq),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - sq),
                    (a + 1.0) - (a - 1.0) * cos + sq,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - sq,
                )
            }
        };
        Coeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// Magnitude response in dB at `freq` (for tests and EQ curve display).
    pub fn magnitude_db(&self, sample_rate: f32, freq: f32) -> f32 {
        let w = 2.0 * PI * freq as f64 / sample_rate as f64;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        let mag = ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt();
        (20.0 * mag.max(1e-12).log10()) as f32
    }
}

/// One channel of biquad state.
#[derive(Debug, Clone, Copy, Default)]
pub struct BiquadState {
    z1: f64,
    z2: f64,
}

impl BiquadState {
    #[inline]
    pub fn tick(&mut self, c: &Coeffs, x: f32) -> f32 {
        let x = x as f64;
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        // flush denormals
        if self.z1.abs() < 1e-30 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-30 {
            self.z2 = 0.0;
        }
        y as f32
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// A stereo biquad with shared coefficients.
#[derive(Debug, Clone, Copy)]
pub struct StereoBiquad {
    pub coeffs: Coeffs,
    l: BiquadState,
    r: BiquadState,
}

impl Default for StereoBiquad {
    fn default() -> Self {
        Self {
            coeffs: Coeffs::IDENTITY,
            l: BiquadState::default(),
            r: BiquadState::default(),
        }
    }
}

impl StereoBiquad {
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let c = self.coeffs;
        for s in left.iter_mut() {
            *s = self.l.tick(&c, *s);
        }
        for s in right.iter_mut() {
            *s = self.r.tick(&c, *s);
        }
    }

    pub fn reset(&mut self) {
        self.l.reset();
        self.r.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    #[test]
    fn peak_gain_at_centre() {
        let c = Coeffs::design(FilterShape::Peak, SR, 1000.0, 1.0, 6.0);
        assert!((c.magnitude_db(SR, 1000.0) - 6.0).abs() < 0.01);
        assert!(c.magnitude_db(SR, 50.0).abs() < 0.2);
        assert!(c.magnitude_db(SR, 18_000.0).abs() < 0.2);
    }

    #[test]
    fn highpass_attenuates_lows() {
        let c = Coeffs::design(FilterShape::HighPass, SR, 100.0, 0.707, 0.0);
        assert!((c.magnitude_db(SR, 100.0) + 3.0).abs() < 0.1);
        assert!(c.magnitude_db(SR, 25.0) < -20.0);
        assert!(c.magnitude_db(SR, 2000.0).abs() < 0.1);
    }

    #[test]
    fn shelves() {
        let lo = Coeffs::design(FilterShape::LowShelf, SR, 200.0, 0.707, -6.0);
        assert!((lo.magnitude_db(SR, 20.0) + 6.0).abs() < 0.3);
        assert!(lo.magnitude_db(SR, 8000.0).abs() < 0.2);
        let hi = Coeffs::design(FilterShape::HighShelf, SR, 6000.0, 0.707, 4.0);
        assert!((hi.magnitude_db(SR, 20_000.0) - 4.0).abs() < 0.4);
        assert!(hi.magnitude_db(SR, 100.0).abs() < 0.1);
    }

    #[test]
    fn time_domain_matches_magnitude() {
        // Filter a sine and compare output amplitude to the design response.
        let c = Coeffs::design(FilterShape::Peak, SR, 2000.0, 2.0, -9.0);
        let mut st = BiquadState::default();
        let mut peak = 0.0f32;
        for i in 0..48_000 {
            let x = (2.0 * std::f32::consts::PI * 2000.0 * i as f32 / SR).sin();
            let y = st.tick(&c, x);
            if i > 24_000 {
                peak = peak.max(y.abs());
            }
        }
        let measured = 20.0 * peak.log10();
        assert!((measured + 9.0).abs() < 0.1, "{measured}");
    }
}
