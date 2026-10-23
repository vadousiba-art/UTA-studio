//! Streaming pitch tracking for live displays.
//!
//! Runs on a worker thread fed from the audio engine's analysis ring buffer —
//! never on the audio callback itself. Applies light median smoothing so the
//! live needle does not jitter, at the cost of ~20 ms extra delay.

use crate::pitch::{PitchConfig, PitchFrame, PitchTracker};
use crate::stats::median;

pub struct LivePitch {
    tracker: PitchTracker,
    buf: Vec<f32>,
    frame_len: usize,
    since_hop: usize,
    hop: usize,
    samples_seen: u64,
    sample_rate: f32,
    recent: [Option<f32>; 5],
    recent_idx: usize,
}

impl LivePitch {
    pub fn new(cfg: PitchConfig) -> Self {
        let tracker = PitchTracker::new(cfg);
        let n = tracker.frame_len();
        Self {
            hop: cfg.hop_samples(),
            sample_rate: cfg.sample_rate as f32,
            tracker,
            buf: Vec::with_capacity(5 * n),
            frame_len: n,
            since_hop: 0,
            samples_seen: 0,
            recent: [None; 5],
            recent_idx: 0,
        }
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.since_hop = 0;
        self.recent = [None; 5];
    }

    /// Push mono samples; `on_frame` is called for every completed hop with a
    /// smoothed frame (its `hz` is the median of the last 5 voiced estimates).
    pub fn push(&mut self, samples: &[f32], mut on_frame: impl FnMut(PitchFrame)) {
        let n = self.frame_len;
        for &s in samples {
            self.buf.push(s);
            self.samples_seen += 1;
            self.since_hop += 1;
            if self.buf.len() >= n && self.since_hop >= self.hop {
                self.since_hop = 0;
                let start = self.buf.len() - n;
                let time = self.samples_seen as f32 / self.sample_rate;
                let mut f = self.tracker.frame(&self.buf[start..], time);
                self.recent[self.recent_idx] = f.hz;
                self.recent_idx = (self.recent_idx + 1) % self.recent.len();
                if f.hz.is_some() {
                    let vals: Vec<f32> = self.recent.iter().flatten().copied().collect();
                    f.hz = median(&vals);
                }
                on_frame(f);
                // keep only what the next frame needs (amortised O(1) per sample)
                if self.buf.len() > 4 * n {
                    self.buf.drain(..self.buf.len() - n);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;

    #[test]
    fn live_tracks_tone_in_small_chunks() {
        let sr = 48_000;
        let sig = synth::harmonic_tone(330.0, &[1.0, 0.6], 1.0, sr);
        let mut live = LivePitch::new(PitchConfig::new(sr));
        let mut frames = Vec::new();
        for chunk in sig.chunks(256) {
            live.push(chunk, |f| frames.push(f));
        }
        assert!(frames.len() > 80, "{}", frames.len());
        let voiced: Vec<f32> = frames.iter().filter_map(|f| f.hz).collect();
        assert!(voiced.len() > 70);
        for hz in voiced {
            assert!((uta_dsp::tone::cents_between(hz, 330.0)).abs() < 5.0);
        }
    }
}
