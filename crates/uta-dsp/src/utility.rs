//! Gain / width / polarity utility.

use crate::effect::{param as p, BuiltinKind, Effect, ParamInfo, ParamScale, ParamValues};
use crate::level::db_to_gain;
use crate::smoothing::Smoother;

pub static PARAMS: &[ParamInfo] = &[
    p("gain", "Gain", "dB", -36.0, 24.0, 0.0, ParamScale::Linear),
    p("width", "Stereo Width", "%", 0.0, 200.0, 100.0, ParamScale::Linear),
    p("invert", "Invert Polarity", "", 0.0, 1.0, 0.0, ParamScale::Toggle),
];

const N: usize = 3;

pub struct Utility {
    params: ParamValues<N>,
    gain: Smoother,
}

impl Default for Utility {
    fn default() -> Self {
        Self::new()
    }
}

impl Utility {
    pub fn new() -> Self {
        Self {
            params: ParamValues::new(PARAMS),
            gain: Smoother::new(1.0),
        }
    }
}

impl Effect for Utility {
    fn kind(&self) -> BuiltinKind {
        BuiltinKind::Utility
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.gain.set_time(sample_rate, 20.0);
        self.gain.snap(db_to_gain(self.params.values[0]));
    }

    fn reset(&mut self) {}

    fn set_param(&mut self, index: usize, value: f32) {
        self.params.set(index, value);
        if index == 0 {
            self.gain.set_target(db_to_gain(self.params.values[0]));
        }
    }

    fn get_param(&self, index: usize) -> f32 {
        self.params.get(index)
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let width = self.params.values[1] / 100.0;
        let sign = if self.params.values[2] >= 0.5 { -1.0 } else { 1.0 };
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let g = self.gain.next() * sign;
            let mid = (*l + *r) * 0.5;
            let side = (*l - *r) * 0.5 * width;
            *l = (mid + side) * g;
            *r = (mid - side) * g;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_and_mono() {
        let mut u = Utility::new();
        u.set_param(0, -6.0206);
        u.set_param(1, 0.0);
        u.prepare(48_000.0, 16);
        let mut l = vec![1.0f32; 16];
        let mut r = vec![0.0f32; 16];
        u.process(&mut l, &mut r);
        assert!((l[15] - 0.25).abs() < 1e-3 && (r[15] - 0.25).abs() < 1e-3);
    }
}
