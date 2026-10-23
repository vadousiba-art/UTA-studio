//! Noise gate / downward expander with hold and hysteresis.

use crate::effect::{param as p, BuiltinKind, Effect, ParamInfo, ParamScale, ParamValues};
use crate::level::{db_to_gain, gain_to_db};

pub static PARAMS: &[ParamInfo] = &[
    p("threshold", "Threshold", "dB", -80.0, 0.0, -50.0, ParamScale::Linear),
    p("range", "Range", "dB", 0.0, 80.0, 30.0, ParamScale::Linear),
    p("attack", "Attack", "ms", 0.1, 50.0, 1.0, ParamScale::Log),
    p("hold", "Hold", "ms", 0.0, 500.0, 50.0, ParamScale::Linear),
    p("release", "Release", "ms", 5.0, 1000.0, 150.0, ParamScale::Log),
];

const N: usize = 5;
/// The gate closes this many dB below the opening threshold.
const HYSTERESIS_DB: f32 = 4.0;

pub struct Gate {
    params: ParamValues<N>,
    sample_rate: f32,
    env: f32,
    gain: f32,
    open: bool,
    hold_left: u32,
    attack_coeff: f32,
    release_coeff: f32,
    env_release: f32,
    last_gr: f32,
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

impl Gate {
    pub fn new() -> Self {
        let mut g = Self {
            params: ParamValues::new(PARAMS),
            sample_rate: 48_000.0,
            env: 0.0,
            gain: 1.0,
            open: false,
            hold_left: 0,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            env_release: 0.0,
            last_gr: 0.0,
        };
        g.update_times();
        g
    }

    fn update_times(&mut self) {
        let t = |ms: f32| (-1.0 / (ms.max(0.01) * 0.001 * self.sample_rate)).exp();
        self.attack_coeff = t(self.params.values[2]);
        self.release_coeff = t(self.params.values[4]);
        self.env_release = t(10.0);
    }
}

impl Effect for Gate {
    fn kind(&self) -> BuiltinKind {
        BuiltinKind::Gate
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sample_rate = sample_rate;
        self.update_times();
        self.reset();
    }

    fn reset(&mut self) {
        self.env = 0.0;
        self.gain = db_to_gain(-self.params.values[1]);
        self.open = false;
        self.hold_left = 0;
        self.last_gr = 0.0;
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if self.params.set(index, value) {
            self.update_times();
        }
    }

    fn get_param(&self, index: usize) -> f32 {
        self.params.get(index)
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let open_th = self.params.values[0];
        let close_th = open_th - HYSTERESIS_DB;
        let floor = db_to_gain(-self.params.values[1]);
        let hold = (self.params.values[3] * 0.001 * self.sample_rate) as u32;
        let mut min_gain = 1.0f32;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let x = l.abs().max(r.abs());
            // instant attack, ~10 ms release envelope follower
            self.env = if x > self.env {
                x
            } else {
                x + self.env_release * (self.env - x)
            };
            let level = gain_to_db(self.env);
            if level >= open_th {
                self.open = true;
                self.hold_left = hold;
            } else if level < close_th {
                if self.hold_left > 0 {
                    self.hold_left -= 1;
                } else {
                    self.open = false;
                }
            }
            let target = if self.open { 1.0 } else { floor };
            let coeff = if target > self.gain {
                self.attack_coeff
            } else {
                self.release_coeff
            };
            self.gain = target + coeff * (self.gain - target);
            min_gain = min_gain.min(self.gain);
            *l *= self.gain;
            *r *= self.gain;
        }
        self.last_gr = -gain_to_db(min_gain);
    }

    fn gain_reduction_db(&self) -> f32 {
        self.last_gr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attenuates_quiet_noise_and_passes_loud_signal() {
        let mut g = Gate::new();
        g.prepare(48_000.0, 4800);
        // quiet noise at about -70 dBFS
        let mut l: Vec<f32> = (0..48_000).map(|i| if i % 2 == 0 { 3e-4 } else { -3e-4 }).collect();
        let mut r = l.clone();
        g.process(&mut l, &mut r);
        let tail = l[40_000..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(gain_to_db(tail) < -95.0, "{}", gain_to_db(tail));

        // loud tone passes almost unchanged after attack
        let mut l: Vec<f32> = (0..48_000)
            .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 300.0 * i as f32 / 48_000.0).sin())
            .collect();
        let mut r = l.clone();
        g.process(&mut l, &mut r);
        let peak = l[24_000..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.3).abs() < 0.01, "{peak}");
    }
}
