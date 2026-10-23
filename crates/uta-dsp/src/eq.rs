//! Five-band vocal EQ: high-pass, low shelf, two peaking bands, high shelf.

use crate::biquad::{Coeffs, FilterShape, StereoBiquad};
use crate::effect::{param as p, BuiltinKind, Effect, ParamInfo, ParamScale, ParamValues};
use crate::level::db_to_gain;
use crate::smoothing::Smoother;

pub static PARAMS: &[ParamInfo] = &[
    p("hpf_on", "Low Cut", "", 0.0, 1.0, 1.0, ParamScale::Toggle),
    p("hpf_freq", "Low Cut Freq", "Hz", 20.0, 400.0, 80.0, ParamScale::Log),
    p("low_freq", "Low Freq", "Hz", 40.0, 500.0, 120.0, ParamScale::Log),
    p("low_gain", "Low Gain", "dB", -18.0, 18.0, 0.0, ParamScale::Linear),
    p("mid1_freq", "Mid 1 Freq", "Hz", 100.0, 5000.0, 400.0, ParamScale::Log),
    p("mid1_gain", "Mid 1 Gain", "dB", -18.0, 18.0, 0.0, ParamScale::Linear),
    p("mid1_q", "Mid 1 Q", "", 0.2, 10.0, 1.0, ParamScale::Log),
    p("mid2_freq", "Mid 2 Freq", "Hz", 500.0, 12000.0, 3000.0, ParamScale::Log),
    p("mid2_gain", "Mid 2 Gain", "dB", -18.0, 18.0, 0.0, ParamScale::Linear),
    p("mid2_q", "Mid 2 Q", "", 0.2, 10.0, 1.0, ParamScale::Log),
    p("high_freq", "High Freq", "Hz", 2000.0, 18000.0, 10000.0, ParamScale::Log),
    p("high_gain", "High Gain", "dB", -18.0, 18.0, 0.0, ParamScale::Linear),
    p("output", "Output", "dB", -18.0, 18.0, 0.0, ParamScale::Linear),
];

const N: usize = 13;

pub struct Equalizer {
    params: ParamValues<N>,
    sample_rate: f32,
    bands: [StereoBiquad; 5],
    dirty: bool,
    output: Smoother,
}

impl Default for Equalizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Equalizer {
    pub fn new() -> Self {
        let mut eq = Self {
            params: ParamValues::new(PARAMS),
            sample_rate: 48_000.0,
            bands: [StereoBiquad::default(); 5],
            dirty: true,
            output: Smoother::new(1.0),
        };
        eq.update_coeffs();
        eq
    }

    fn update_coeffs(&mut self) {
        let v = &self.params.values;
        let sr = self.sample_rate;
        self.bands[0].coeffs = if v[0] >= 0.5 {
            Coeffs::design(FilterShape::HighPass, sr, v[1], 0.707, 0.0)
        } else {
            Coeffs::IDENTITY
        };
        self.bands[1].coeffs = Coeffs::design(FilterShape::LowShelf, sr, v[2], 0.707, v[3]);
        self.bands[2].coeffs = Coeffs::design(FilterShape::Peak, sr, v[4], v[6], v[5]);
        self.bands[3].coeffs = Coeffs::design(FilterShape::Peak, sr, v[7], v[9], v[8]);
        self.bands[4].coeffs = Coeffs::design(FilterShape::HighShelf, sr, v[10], 0.707, v[11]);
        self.output.set_target(db_to_gain(v[12]));
        self.dirty = false;
    }

    /// Combined magnitude response in dB (for drawing the EQ curve).
    pub fn response_db(&mut self, freq: f32) -> f32 {
        if self.dirty {
            self.update_coeffs();
        }
        self.bands
            .iter()
            .map(|b| b.coeffs.magnitude_db(self.sample_rate, freq))
            .sum::<f32>()
            + self.params.values[12]
    }
}

impl Effect for Equalizer {
    fn kind(&self) -> BuiltinKind {
        BuiltinKind::Eq
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        self.output.set_time(sample_rate, 20.0);
        self.update_coeffs();
        self.output.snap(self.output.target());
        self.reset();
    }

    fn reset(&mut self) {
        for b in &mut self.bands {
            b.reset();
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if self.params.set(index, value) {
            self.dirty = true;
        }
    }

    fn get_param(&self, index: usize) -> f32 {
        self.params.get(index)
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.dirty {
            self.update_coeffs();
        }
        let v = &self.params.values;
        // Skip bands that are flat to save CPU.
        let active = [v[0] >= 0.5, v[3] != 0.0, v[5] != 0.0, v[8] != 0.0, v[11] != 0.0];
        for (band, on) in self.bands.iter_mut().zip(active) {
            if on {
                band.process(left, right);
            }
        }
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let g = self.output.next();
            *l *= g;
            *r *= g;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::param_index;

    #[test]
    fn default_is_flat_above_low_cut() {
        let mut eq = Equalizer::new();
        eq.prepare(48_000.0, 256);
        for f in [500.0, 1000.0, 5000.0, 12_000.0] {
            assert!(eq.response_db(f).abs() < 0.2, "{f}");
        }
        assert!(eq.response_db(30.0) < -6.0);
    }

    #[test]
    fn boost_mid() {
        let mut eq = Equalizer::new();
        eq.prepare(48_000.0, 256);
        eq.set_param(param_index(PARAMS, "mid2_gain").unwrap(), 6.0);
        eq.set_param(param_index(PARAMS, "mid2_freq").unwrap(), 3000.0);
        assert!((eq.response_db(3000.0) - 6.0).abs() < 0.2);
    }
}
