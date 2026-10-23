//! Algorithmic reverb based on the public-domain Freeverb topology
//! (Jezar at Dreampoint): 8 parallel low-pass-feedback comb filters followed by
//! 4 series all-pass filters per channel, with a pre-delay line.

use crate::effect::{param as p, BuiltinKind, Effect, ParamInfo, ParamScale, ParamValues};

pub static PARAMS: &[ParamInfo] = &[
    p("size", "Room Size", "%", 0.0, 100.0, 55.0, ParamScale::Linear),
    p("damping", "Damping", "%", 0.0, 100.0, 50.0, ParamScale::Linear),
    p("predelay", "Pre-delay", "ms", 0.0, 200.0, 20.0, ParamScale::Linear),
    p("width", "Width", "%", 0.0, 100.0, 100.0, ParamScale::Linear),
    p("mix", "Mix", "%", 0.0, 100.0, 20.0, ParamScale::Linear),
];

const N: usize = 5;
const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const FIXED_GAIN: f32 = 0.015;
const MAX_PREDELAY_MS: f32 = 200.0;

#[derive(Default)]
struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            idx: 0,
            store: 0.0,
        }
    }
    #[inline]
    fn tick(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        self.store = out * (1.0 - damp) + self.store * damp;
        if self.store.abs() < 1e-20 {
            self.store = 0.0;
        }
        self.buf[self.idx] = x + self.store * feedback;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.store = 0.0;
    }
}

#[derive(Default)]
struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            idx: 0,
        }
    }
    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let b = self.buf[self.idx];
        let out = b - x;
        let mut v = x + b * 0.5;
        if v.abs() < 1e-20 {
            v = 0.0;
        }
        self.buf[self.idx] = v;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

pub struct Reverb {
    params: ParamValues<N>,
    sample_rate: f32,
    combs_l: Vec<Comb>,
    combs_r: Vec<Comb>,
    aps_l: Vec<Allpass>,
    aps_r: Vec<Allpass>,
    pre: Vec<f32>,
    pre_idx: usize,
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

impl Reverb {
    pub fn new() -> Self {
        let mut r = Self {
            params: ParamValues::new(PARAMS),
            sample_rate: 0.0,
            combs_l: Vec::new(),
            combs_r: Vec::new(),
            aps_l: Vec::new(),
            aps_r: Vec::new(),
            pre: Vec::new(),
            pre_idx: 0,
        };
        r.allocate(48_000.0);
        r
    }

    fn allocate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() < 0.5 && !self.combs_l.is_empty() {
            return;
        }
        self.sample_rate = sample_rate;
        let scale = sample_rate / 44_100.0;
        let s = |n: usize| ((n as f32) * scale).round() as usize;
        self.combs_l = COMB_TUNING.iter().map(|&n| Comb::new(s(n))).collect();
        self.combs_r = COMB_TUNING
            .iter()
            .map(|&n| Comb::new(s(n + STEREO_SPREAD)))
            .collect();
        self.aps_l = ALLPASS_TUNING.iter().map(|&n| Allpass::new(s(n))).collect();
        self.aps_r = ALLPASS_TUNING
            .iter()
            .map(|&n| Allpass::new(s(n + STEREO_SPREAD)))
            .collect();
        self.pre = vec![0.0; (MAX_PREDELAY_MS * 0.001 * sample_rate) as usize + 1];
        self.pre_idx = 0;
    }
}

impl Effect for Reverb {
    fn kind(&self) -> BuiltinKind {
        BuiltinKind::Reverb
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.allocate(sample_rate);
        self.reset();
    }

    fn reset(&mut self) {
        self.combs_l.iter_mut().for_each(Comb::clear);
        self.combs_r.iter_mut().for_each(Comb::clear);
        self.aps_l.iter_mut().for_each(Allpass::clear);
        self.aps_r.iter_mut().for_each(Allpass::clear);
        self.pre.fill(0.0);
        self.pre_idx = 0;
    }

    fn set_param(&mut self, index: usize, value: f32) {
        self.params.set(index, value);
    }

    fn get_param(&self, index: usize) -> f32 {
        self.params.get(index)
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let v = self.params.values;
        let feedback = (v[0] / 100.0) * 0.28 + 0.7;
        let damp = (v[1] / 100.0) * 0.4;
        let width = v[3] / 100.0;
        let mix = v[4] / 100.0;
        let wet = mix * 3.0;
        let wet1 = wet * (width / 2.0 + 0.5);
        let wet2 = wet * ((1.0 - width) / 2.0);
        let dry = 1.0 - mix;
        let pre_len = self.pre.len();
        let pre_delay = ((v[2] * 0.001 * self.sample_rate) as usize).min(pre_len - 1);

        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let input = (*l + *r) * FIXED_GAIN;
            // pre-delay
            self.pre[self.pre_idx] = input;
            let read = (self.pre_idx + pre_len - pre_delay) % pre_len;
            let x = self.pre[read];
            self.pre_idx = (self.pre_idx + 1) % pre_len;

            let mut out_l = 0.0;
            let mut out_r = 0.0;
            for c in &mut self.combs_l {
                out_l += c.tick(x, feedback, damp);
            }
            for c in &mut self.combs_r {
                out_r += c.tick(x, feedback, damp);
            }
            for a in &mut self.aps_l {
                out_l = a.tick(out_l);
            }
            for a in &mut self.aps_r {
                out_r = a.tick(out_r);
            }
            let dl = *l;
            let dr = *r;
            *l = out_l * wet1 + out_r * wet2 + dl * dry;
            *r = out_r * wet1 + out_l * wet2 + dr * dry;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_produces_decaying_tail() {
        let mut rv = Reverb::new();
        rv.prepare(48_000.0, 48_000);
        rv.set_param(4, 100.0); // fully wet
        rv.set_param(2, 0.0);
        let mut l = vec![0.0f32; 48_000];
        let mut r = vec![0.0f32; 48_000];
        l[0] = 1.0;
        r[0] = 1.0;
        rv.process(&mut l, &mut r);
        let energy = |s: &[f32]| s.iter().map(|x| x * x).sum::<f32>();
        let early = energy(&l[2000..12_000]);
        let late = energy(&l[38_000..48_000]);
        assert!(early > 0.0, "tail exists");
        assert!(late < early, "tail decays: {early} {late}");
        // stereo decorrelation
        assert!(l[5000..6000] != r[5000..6000]);
    }

    #[test]
    fn dry_when_mix_zero() {
        let mut rv = Reverb::new();
        rv.prepare(44_100.0, 128);
        rv.set_param(4, 0.0);
        let mut l = vec![0.25f32; 128];
        let mut r = vec![-0.25f32; 128];
        rv.process(&mut l, &mut r);
        assert!(l.iter().all(|&s| (s - 0.25).abs() < 1e-6));
        assert!(r.iter().all(|&s| (s + 0.25).abs() < 1e-6));
    }
}
