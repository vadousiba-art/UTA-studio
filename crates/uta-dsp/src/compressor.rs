//! Feed-forward, stereo-linked compressor with a soft knee.
//!
//! Gain computer and log-domain smoothing follow Giannoulis, Massberg & Reiss,
//! "Digital Dynamic Range Compressor Design — A Tutorial and Analysis"
//! (JAES 2012): smoothed decoupled peak detector on the gain-reduction signal.

use crate::effect::{param as p, BuiltinKind, Effect, ParamInfo, ParamScale, ParamValues};
use crate::level::{db_to_gain, gain_to_db};

pub static PARAMS: &[ParamInfo] = &[
    p("threshold", "Threshold", "dB", -60.0, 0.0, -18.0, ParamScale::Linear),
    p("ratio", "Ratio", ":1", 1.0, 20.0, 3.0, ParamScale::Log),
    p("attack", "Attack", "ms", 0.1, 100.0, 10.0, ParamScale::Log),
    p("release", "Release", "ms", 10.0, 1000.0, 120.0, ParamScale::Log),
    p("knee", "Knee", "dB", 0.0, 12.0, 6.0, ParamScale::Linear),
    p("makeup", "Makeup", "dB", 0.0, 24.0, 0.0, ParamScale::Linear),
    p("mix", "Mix", "%", 0.0, 100.0, 100.0, ParamScale::Linear),
];

const N: usize = 7;

pub struct Compressor {
    params: ParamValues<N>,
    sample_rate: f32,
    attack_coeff: f32,
    release_coeff: f32,
    /// Smoothed gain reduction in dB (>= 0).
    gr_db: f32,
    last_gr: f32,
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compressor {
    pub fn new() -> Self {
        let mut c = Self {
            params: ParamValues::new(PARAMS),
            sample_rate: 48_000.0,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            gr_db: 0.0,
            last_gr: 0.0,
        };
        c.update_times();
        c
    }

    fn update_times(&mut self) {
        let t = |ms: f32| (-1.0 / (ms * 0.001 * self.sample_rate)).exp();
        self.attack_coeff = t(self.params.values[2]);
        self.release_coeff = t(self.params.values[3]);
    }

    /// Static gain computer: input level (dB) -> gain reduction (dB, >= 0).
    pub fn static_gain_reduction(&self, level_db: f32) -> f32 {
        let threshold = self.params.values[0];
        let ratio = self.params.values[1];
        let knee = self.params.values[4];
        let over = level_db - threshold;
        let slope = 1.0 - 1.0 / ratio;
        if knee > 0.0 && 2.0 * over.abs() <= knee {
            slope * (over + knee / 2.0).powi(2) / (2.0 * knee)
        } else if over > 0.0 {
            slope * over
        } else {
            0.0
        }
    }
}

impl Effect for Compressor {
    fn kind(&self) -> BuiltinKind {
        BuiltinKind::Compressor
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        self.update_times();
        self.reset();
    }

    fn reset(&mut self) {
        self.gr_db = 0.0;
        self.last_gr = 0.0;
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if self.params.set(index, value) && (index == 2 || index == 3) {
            self.update_times();
        }
    }

    fn get_param(&self, index: usize) -> f32 {
        self.params.get(index)
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let makeup = db_to_gain(self.params.values[5]);
        let wet = self.params.values[6] / 100.0;
        let dry = 1.0 - wet;
        let mut max_gr = 0.0f32;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let level = gain_to_db(l.abs().max(r.abs()));
            let target = self.static_gain_reduction(level);
            let coeff = if target > self.gr_db {
                self.attack_coeff
            } else {
                self.release_coeff
            };
            self.gr_db = target + coeff * (self.gr_db - target);
            if self.gr_db < 1e-6 {
                self.gr_db = 0.0;
            }
            max_gr = max_gr.max(self.gr_db);
            let g = db_to_gain(-self.gr_db) * makeup;
            *l = *l * (dry + wet * g);
            *r = *r * (dry + wet * g);
        }
        self.last_gr = max_gr;
    }

    fn gain_reduction_db(&self) -> f32 {
        self.last_gr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::param_index;

    fn sine(amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 48_000.0).sin())
            .collect()
    }

    #[test]
    fn static_curve() {
        let mut c = Compressor::new();
        c.set_param(param_index(PARAMS, "knee").unwrap(), 0.0);
        c.set_param(param_index(PARAMS, "ratio").unwrap(), 4.0);
        c.set_param(param_index(PARAMS, "threshold").unwrap(), -20.0);
        assert_eq!(c.static_gain_reduction(-30.0), 0.0);
        // 12 dB over at 4:1 => 9 dB reduction
        assert!((c.static_gain_reduction(-8.0) - 9.0).abs() < 1e-4);
    }

    #[test]
    fn reduces_loud_signal_and_leaves_quiet_signal() {
        let mut c = Compressor::new();
        c.prepare(48_000.0, 48_000);
        c.set_param(param_index(PARAMS, "threshold").unwrap(), -20.0);
        c.set_param(param_index(PARAMS, "ratio").unwrap(), 4.0);
        c.set_param(param_index(PARAMS, "knee").unwrap(), 0.0);

        let mut l = sine(0.5, 48_000); // ~ -6 dBFS peak
        let mut r = l.clone();
        c.process(&mut l, &mut r);
        let out_peak = l[24_000..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        // expected: -6 dB is 14 dB over, reduced by 10.5 dB -> ~ -16.5 dBFS.
        // Peak-detector ripple on a sine allows some tolerance.
        let out_db = gain_to_db(out_peak);
        assert!(out_db < -14.0 && out_db > -19.0, "{out_db}");
        assert!(c.gain_reduction_db() > 8.0);

        let mut c2 = Compressor::new();
        c2.prepare(48_000.0, 48_000);
        let mut l = sine(0.01, 4800);
        let orig = l.clone();
        let mut r = l.clone();
        c2.process(&mut l, &mut r);
        for (a, b) in l.iter().zip(&orig) {
            assert!((a - b).abs() < 1e-6);
        }
    }
}
