//! Stereo feedback delay with damping in the feedback path, smoothed delay
//! time (no clicks when the time changes) and optional ping-pong.

use crate::effect::{param as p, BuiltinKind, Effect, ParamInfo, ParamScale, ParamValues};
use crate::smoothing::Smoother;

pub static PARAMS: &[ParamInfo] = &[
    p("time", "Time", "ms", 10.0, 2000.0, 350.0, ParamScale::Log),
    p("feedback", "Feedback", "%", 0.0, 95.0, 30.0, ParamScale::Linear),
    p("mix", "Mix", "%", 0.0, 100.0, 20.0, ParamScale::Linear),
    p("highcut", "High Cut", "Hz", 1000.0, 20000.0, 8000.0, ParamScale::Log),
    p("pingpong", "Ping-Pong", "", 0.0, 1.0, 0.0, ParamScale::Toggle),
];

const N: usize = 5;
const MAX_TIME_MS: f32 = 2000.0;

pub struct Delay {
    params: ParamValues<N>,
    sample_rate: f32,
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    write: usize,
    time: Smoother,
    lp_l: f32,
    lp_r: f32,
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

impl Delay {
    pub fn new() -> Self {
        let mut d = Self {
            params: ParamValues::new(PARAMS),
            sample_rate: 0.0,
            buf_l: Vec::new(),
            buf_r: Vec::new(),
            write: 0,
            time: Smoother::new(0.0),
            lp_l: 0.0,
            lp_r: 0.0,
        };
        d.allocate(48_000.0);
        d
    }

    fn allocate(&mut self, sr: f32) {
        if (self.sample_rate - sr).abs() < 0.5 && !self.buf_l.is_empty() {
            return;
        }
        self.sample_rate = sr;
        let len = (MAX_TIME_MS * 0.001 * sr) as usize + 4;
        self.buf_l = vec![0.0; len];
        self.buf_r = vec![0.0; len];
        self.write = 0;
        self.time.set_time(sr, 60.0);
        self.time.snap(self.params.values[0] * 0.001 * sr);
    }

    #[inline]
    fn read(buf: &[f32], write: usize, delay: f32) -> f32 {
        let len = buf.len();
        let d = delay.clamp(1.0, (len - 2) as f32);
        let di = d.floor() as usize;
        let frac = d - di as f32;
        let i0 = (write + len - di) % len;
        let i1 = (i0 + len - 1) % len;
        buf[i0] * (1.0 - frac) + buf[i1] * frac
    }
}

impl Effect for Delay {
    fn kind(&self) -> BuiltinKind {
        BuiltinKind::Delay
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.allocate(sample_rate);
        self.reset();
    }

    fn reset(&mut self) {
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.lp_l = 0.0;
        self.lp_r = 0.0;
        self.time.snap(self.params.values[0] * 0.001 * self.sample_rate);
    }

    fn set_param(&mut self, index: usize, value: f32) {
        self.params.set(index, value);
        if index == 0 {
            self.time
                .set_target(self.params.values[0] * 0.001 * self.sample_rate);
        }
    }

    fn get_param(&self, index: usize) -> f32 {
        self.params.get(index)
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let v = self.params.values;
        let fb = v[1] / 100.0;
        let mix = v[2] / 100.0;
        let lp = 1.0 - (-2.0 * std::f32::consts::PI * v[3] / self.sample_rate).exp();
        let pingpong = v[4] >= 0.5;
        let len = self.buf_l.len();
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let d = self.time.next();
            let tl = Self::read(&self.buf_l, self.write, d);
            let tr = Self::read(&self.buf_r, self.write, d);
            self.lp_l += lp * (tl - self.lp_l);
            self.lp_r += lp * (tr - self.lp_r);
            if self.lp_l.abs() < 1e-20 {
                self.lp_l = 0.0;
            }
            if self.lp_r.abs() < 1e-20 {
                self.lp_r = 0.0;
            }
            let (in_l, in_r) = if pingpong {
                // mono input enters left; feedback crosses channels
                ((*l + *r) * 0.5 + self.lp_r * fb, self.lp_l * fb)
            } else {
                (*l + self.lp_l * fb, *r + self.lp_r * fb)
            };
            self.buf_l[self.write] = in_l;
            self.buf_r[self.write] = in_r;
            self.write = (self.write + 1) % len;
            *l = *l * (1.0 - mix) + tl * mix;
            *r = *r * (1.0 - mix) + tr * mix;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_at_expected_time() {
        let mut d = Delay::new();
        d.prepare(48_000.0, 48_000);
        d.set_param(0, 100.0); // 100 ms
        d.reset();
        d.set_param(1, 0.0);
        d.set_param(2, 100.0);
        d.set_param(3, 20_000.0);
        let mut l = vec![0.0f32; 48_000];
        let mut r = vec![0.0f32; 48_000];
        l[0] = 1.0;
        r[0] = 1.0;
        d.process(&mut l, &mut r);
        let (pos, _) = l
            .iter()
            .enumerate()
            .fold((0, 0.0f32), |(bi, bv), (i, &v)| if v.abs() > bv { (i, v.abs()) } else { (bi, bv) });
        assert!((pos as i64 - 4800).abs() <= 2, "{pos}");
    }

    #[test]
    fn feedback_repeats_decay() {
        let mut d = Delay::new();
        d.prepare(48_000.0, 48_000);
        d.set_param(0, 50.0);
        d.reset();
        d.set_param(1, 50.0);
        d.set_param(2, 100.0);
        d.set_param(3, 20_000.0);
        let mut l = vec![0.0f32; 48_000];
        let mut r = vec![0.0f32; 48_000];
        l[0] = 1.0;
        d.process(&mut l, &mut r);
        let e1 = l[2390..2410].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let e2 = l[4790..4810].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(e1 > 0.5 && e2 > 0.2 && e2 < e1, "{e1} {e2}");
    }
}
