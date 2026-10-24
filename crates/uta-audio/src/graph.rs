//! The mixing graph: per-track effect chains, gain, pan, mute/solo, input
//! monitoring and meters, summed into a master bus with its own chain.
//!
//! The same [`Graph`] code runs on the audio thread (fed from streaming ring
//! buffers) and in the offline renderer (fed directly from files), so exports
//! sound exactly like playback. `process` never allocates: all scratch
//! buffers are sized for [`MAX_BLOCK`] when the graph is built.

use uta_dsp::smoothing::Smoother;
use uta_dsp::Effect;

use crate::shared::SharedState;

/// Largest number of frames processed in one call.
pub const MAX_BLOCK: usize = 512;

/// Anything that can sit in an effect chain: built-in effects and hosted
/// plugins.
pub trait ChainProcessor: Send {
    /// Process a block in place.
    fn process(&mut self, left: &mut [f32], right: &mut [f32]);
    /// Set a parameter by index (built-ins) or plugin parameter id.
    fn set_param(&mut self, index: u32, value: f32);
    /// Clear internal state (tails) without reallocating.
    fn reset(&mut self);
    fn latency_samples(&self) -> usize {
        0
    }
    fn gain_reduction_db(&self) -> f32 {
        0.0
    }
    /// Called instead of `process` while the slot is bypassed. Processors
    /// with latency must still delay the signal by `latency_samples` so the
    /// track stays aligned; the default (zero latency) leaves it untouched.
    fn process_bypassed(&mut self, _left: &mut [f32], _right: &mut [f32]) {}
}

/// A fixed stereo delay used for plugin delay compensation. The buffer is
/// allocated up front; `process` never allocates.
pub struct FixedDelay {
    l: Vec<f32>,
    r: Vec<f32>,
    pos: usize,
    delay: usize,
}

impl FixedDelay {
    pub fn new(delay: usize) -> Self {
        let len = if delay == 0 { 0 } else { delay + 1 };
        Self { l: vec![0.0; len], r: vec![0.0; len], pos: 0, delay }
    }

    pub fn delay(&self) -> usize {
        self.delay
    }

    #[inline]
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.delay == 0 {
            return;
        }
        let n = self.l.len();
        for (a, b) in left.iter_mut().zip(right.iter_mut()) {
            self.l[self.pos] = *a;
            self.r[self.pos] = *b;
            let read = (self.pos + n - self.delay) % n;
            *a = self.l[read];
            *b = self.r[read];
            self.pos = (self.pos + 1) % n;
        }
    }

    pub fn reset(&mut self) {
        self.l.fill(0.0);
        self.r.fill(0.0);
    }
}

/// Adapter for built-in effects.
pub struct BuiltinProcessor(pub Box<dyn Effect>);

impl ChainProcessor for BuiltinProcessor {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.0.process(left, right);
    }
    fn set_param(&mut self, index: u32, value: f32) {
        self.0.set_param(index as usize, value);
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn latency_samples(&self) -> usize {
        self.0.latency_samples()
    }
    fn gain_reduction_db(&self) -> f32 {
        self.0.gain_reduction_db()
    }
}

/// A processor that does nothing — stands in for a missing plugin so the
/// chain keeps its shape and the stored state is preserved.
pub struct PassThrough;

impl ChainProcessor for PassThrough {
    fn process(&mut self, _left: &mut [f32], _right: &mut [f32]) {}
    fn set_param(&mut self, _index: u32, _value: f32) {}
    fn reset(&mut self) {}
}

pub struct ChainSlot {
    pub id: u128,
    pub processor: Box<dyn ChainProcessor>,
    pub bypass: bool,
}

#[derive(Default)]
pub struct Chain {
    pub slots: Vec<ChainSlot>,
}

impl Chain {
    #[inline]
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        for s in &mut self.slots {
            if s.bypass {
                s.processor.process_bypassed(l, r);
            } else {
                s.processor.process(l, r);
            }
        }
    }

    pub fn slot_mut(&mut self, id: u128) -> Option<&mut ChainSlot> {
        self.slots.iter_mut().find(|s| s.id == id)
    }

    pub fn reset(&mut self) {
        for s in &mut self.slots {
            s.processor.reset();
        }
    }

    /// Total latency of the chain. Bypassed slots count too: they keep
    /// delaying the signal so toggling bypass never shifts the track.
    pub fn latency(&self) -> usize {
        self.slots.iter().map(|s| s.processor.latency_samples()).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputRoute {
    pub channel: u16,
    pub stereo: bool,
}

/// Pan law used for all tracks: unity at centre, the far side fades out
/// along a constant-power curve (balance behaviour for stereo material).
#[inline]
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let (l, r) = uta_dsp::constant_power_pan(pan);
    ((l * std::f32::consts::SQRT_2).min(1.0), (r * std::f32::consts::SQRT_2).min(1.0))
}

pub struct GraphTrack {
    pub id: u128,
    /// Index into `SharedState::track_meters`.
    pub meter: usize,
    pub gain: Smoother,
    pub pan: (f32, f32),
    /// Mute/solo resolved: whether the track reaches the master bus.
    pub audible: bool,
    /// Pass the live input through this track.
    pub monitor: bool,
    pub input: InputRoute,
    pub chain: Chain,
    /// Delay compensation so every track lines up with the slowest chain.
    pub compensation: FixedDelay,
    l: Vec<f32>,
    r: Vec<f32>,
}

impl GraphTrack {
    pub fn new(id: u128, meter: usize, sample_rate: f32) -> Self {
        let mut gain = Smoother::new(1.0);
        gain.set_time(sample_rate, 15.0);
        Self {
            id,
            meter,
            gain,
            pan: (1.0, 1.0),
            audible: true,
            monitor: false,
            input: InputRoute { channel: 0, stereo: false },
            chain: Chain::default(),
            compensation: FixedDelay::new(0),
            l: vec![0.0; MAX_BLOCK],
            r: vec![0.0; MAX_BLOCK],
        }
    }

    pub fn set_gain(&mut self, linear: f32, snap: bool) {
        if snap {
            self.gain.snap(linear);
        } else {
            self.gain.set_target(linear);
        }
    }
}

/// Supplies source audio for tracks (streams on the audio thread, file
/// readers offline). Implementations must *write* (not add) `l`/`r`.
pub trait TrackFeed {
    fn fill(&mut self, index: usize, id: u128, l: &mut [f32], r: &mut [f32]);
}

/// A feed that produces silence (transport stopped).
pub struct SilentFeed;

impl TrackFeed for SilentFeed {
    fn fill(&mut self, _index: usize, _id: u128, l: &mut [f32], r: &mut [f32]) {
        l.fill(0.0);
        r.fill(0.0);
    }
}

pub struct Graph {
    pub sample_rate: f32,
    pub tracks: Vec<GraphTrack>,
    pub master: Chain,
    pub master_gain: Smoother,
}

impl Graph {
    pub fn new(sample_rate: f32) -> Self {
        let mut master_gain = Smoother::new(1.0);
        master_gain.set_time(sample_rate, 15.0);
        Self {
            sample_rate,
            tracks: Vec::new(),
            master: Chain::default(),
            master_gain,
        }
    }

    pub fn track_mut(&mut self, id: u128) -> Option<&mut GraphTrack> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    pub fn reset(&mut self) {
        for t in &mut self.tracks {
            t.chain.reset();
            t.compensation.reset();
        }
        self.master.reset();
    }

    /// Give every track a compensation delay so all tracks arrive at the
    /// master bus together (call after the chains are built).
    pub fn compensate_latency(&mut self) {
        let max = self.tracks.iter().map(|t| t.chain.latency()).max().unwrap_or(0);
        for t in &mut self.tracks {
            t.compensation = FixedDelay::new(max - t.chain.latency());
        }
    }

    /// Delay from the track feeds to the graph output, in frames.
    pub fn latency(&self) -> usize {
        self.tracks.iter().map(|t| t.chain.latency() + t.compensation.delay()).max().unwrap_or(0) + self.master.latency()
    }

    /// Mix one block. `inputs` holds de-interleaved live input channels for
    /// monitoring (may be empty). `out_l`/`out_r` are overwritten.
    pub fn process(
        &mut self,
        frames: usize,
        feed: &mut dyn TrackFeed,
        inputs: &[Vec<f32>],
        out_l: &mut [f32],
        out_r: &mut [f32],
        shared: Option<&SharedState>,
    ) {
        debug_assert!(frames <= MAX_BLOCK);
        let out_l = &mut out_l[..frames];
        let out_r = &mut out_r[..frames];
        out_l.fill(0.0);
        out_r.fill(0.0);
        for (i, t) in self.tracks.iter_mut().enumerate() {
            let l = &mut t.l[..frames];
            let r = &mut t.r[..frames];
            feed.fill(i, t.id, l, r);
            if t.monitor && !inputs.is_empty() {
                let c = t.input.channel as usize;
                if let Some(a) = inputs.get(c) {
                    if t.input.stereo {
                        let b = inputs.get(c + 1).unwrap_or(a);
                        for k in 0..frames {
                            l[k] += a[k];
                            r[k] += b[k];
                        }
                    } else {
                        for k in 0..frames {
                            l[k] += a[k];
                            r[k] += a[k];
                        }
                    }
                }
            }
            t.chain.process(l, r);
            t.compensation.process(l, r);
            let (pl, pr) = t.pan;
            for k in 0..frames {
                let g = t.gain.next();
                l[k] *= g * pl;
                r[k] *= g * pr;
            }
            if let Some(s) = shared {
                if let Some(m) = s.track_meters.get(t.meter) {
                    m.update(l, r);
                }
            }
            if t.audible {
                for k in 0..frames {
                    out_l[k] += l[k];
                    out_r[k] += r[k];
                }
            }
        }
        self.master.process(out_l, out_r);
        for k in 0..frames {
            let g = self.master_gain.next();
            out_l[k] *= g;
            out_r[k] *= g;
        }
        if let Some(s) = shared {
            s.master_meter.update(out_l, out_r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ConstFeed(f32);
    impl TrackFeed for ConstFeed {
        fn fill(&mut self, _i: usize, _id: u128, l: &mut [f32], r: &mut [f32]) {
            l.fill(self.0);
            r.fill(self.0);
        }
    }

    fn graph_with_tracks(n: usize) -> Graph {
        let mut g = Graph::new(48_000.0);
        for i in 0..n {
            g.tracks.push(GraphTrack::new(i as u128 + 1, i, 48_000.0));
        }
        g
    }

    #[test]
    fn sums_tracks_with_gain_pan_and_mute() {
        let mut g = graph_with_tracks(3);
        g.tracks[0].set_gain(0.5, true);
        g.tracks[1].pan = pan_gains(1.0); // hard right
        g.tracks[2].audible = false;
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        g.process(64, &mut ConstFeed(0.25), &[], &mut l, &mut r, None);
        // track0: 0.125 both; track1: 0 left, 0.25 right; track2 muted
        assert!((l[10] - 0.125).abs() < 1e-6, "{}", l[10]);
        assert!((r[10] - 0.375).abs() < 1e-6, "{}", r[10]);
    }

    #[test]
    fn monitoring_routes_input_channel() {
        let mut g = graph_with_tracks(1);
        g.tracks[0].monitor = true;
        g.tracks[0].input = InputRoute { channel: 1, stereo: false };
        let inputs = vec![vec![0.0; 32], vec![0.5; 32]];
        let (mut l, mut r) = (vec![0.0; 32], vec![0.0; 32]);
        g.process(32, &mut SilentFeed, &inputs, &mut l, &mut r, None);
        assert_eq!(l[5], 0.5);
        assert_eq!(r[5], 0.5);
    }

    #[test]
    fn bypassed_effect_is_skipped_and_meters_update() {
        let shared = SharedState::new();
        let mut g = graph_with_tracks(1);
        let mut util = uta_dsp::create_builtin(uta_dsp::BuiltinKind::Utility);
        util.set_param_by_id("gain", -6.0206);
        util.prepare(48_000.0, MAX_BLOCK);
        g.tracks[0].chain.slots.push(ChainSlot { id: 9, processor: Box::new(BuiltinProcessor(util)), bypass: true });
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        g.process(64, &mut ConstFeed(0.5), &[], &mut l, &mut r, Some(&shared));
        assert!((l[63] - 0.5).abs() < 1e-6);
        g.tracks[0].chain.slots[0].bypass = false;
        g.process(64, &mut ConstFeed(0.5), &[], &mut l, &mut r, Some(&shared));
        assert!((l[63] - 0.25).abs() < 1e-3);
        let m = shared.track_meters[0].take();
        assert!((m[0].peak - 0.5).abs() < 1e-6);
        assert!(shared.master_meter.take()[0].peak > 0.24);
    }

    struct Delayer(FixedDelay);
    impl ChainProcessor for Delayer {
        fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
            self.0.process(l, r);
        }
        fn process_bypassed(&mut self, l: &mut [f32], r: &mut [f32]) {
            self.0.process(l, r);
        }
        fn set_param(&mut self, _i: u32, _v: f32) {}
        fn reset(&mut self) {
            self.0.reset();
        }
        fn latency_samples(&self) -> usize {
            self.0.delay()
        }
    }

    struct Impulse;
    impl TrackFeed for Impulse {
        fn fill(&mut self, _i: usize, _id: u128, l: &mut [f32], r: &mut [f32]) {
            l.fill(0.0);
            r.fill(0.0);
            l[0] = 1.0;
            r[0] = 1.0;
        }
    }

    #[test]
    fn latency_is_compensated_across_tracks() {
        let mut g = graph_with_tracks(2);
        g.tracks[0].chain.slots.push(ChainSlot { id: 1, processor: Box::new(Delayer(FixedDelay::new(10))), bypass: false });
        g.compensate_latency();
        assert_eq!(g.latency(), 10);
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        g.process(64, &mut Impulse, &[], &mut l, &mut r, None);
        // both impulses arrive together at frame 10
        assert!((l[10] - 2.0).abs() < 1e-6, "{:?}", &l[..16]);
        assert_eq!(l.iter().filter(|v| v.abs() > 1e-6).count(), 1);
        // bypassing keeps the alignment
        g.tracks[0].chain.slots[0].bypass = true;
        g.reset();
        g.process(64, &mut Impulse, &[], &mut l, &mut r, None);
        assert!((l[10] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn pan_law_unity_centre() {
        let (l, r) = pan_gains(0.0);
        assert!((l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        let (l, r) = pan_gains(-1.0);
        assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6);
    }
}
