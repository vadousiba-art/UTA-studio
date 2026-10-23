//! Live pitch worker: consumes the engine's analysis ring on a normal
//! thread and keeps a bounded buffer of recent frames for the UI.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;
use rtrb::Consumer;
use serde::Serialize;
use uta_analysis::live::LivePitch;
use uta_analysis::pitch::{PitchConfig, PitchFrame};

const KEEP: usize = 1200;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveFrame {
    pub seq: u64,
    #[serde(flatten)]
    pub frame: PitchFrame,
}

pub struct LiveWorker {
    pub enabled: Arc<AtomicBool>,
    frames: Arc<Mutex<VecDeque<LiveFrame>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    seq: Arc<AtomicU64>,
}

impl LiveWorker {
    pub fn spawn(mut ring: Consumer<f32>, sample_rate: u32, hop_ms: f32) -> Self {
        let enabled = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(Mutex::new(VecDeque::with_capacity(KEEP)));
        let stop = Arc::new(AtomicBool::new(false));
        let seq = Arc::new(AtomicU64::new(0));
        let (en, fr, st, sq) = (enabled.clone(), frames.clone(), stop.clone(), seq.clone());
        let handle = std::thread::Builder::new()
            .name("uta-live-pitch".into())
            .spawn(move || {
                let mut cfg = PitchConfig::new(sample_rate);
                cfg.hop_seconds = (hop_ms / 1000.0).clamp(0.005, 0.05);
                let mut live = LivePitch::new(cfg);
                let mut buf = Vec::with_capacity(8192);
                let mut was_enabled = false;
                while !st.load(Ordering::Acquire) {
                    let n = ring.slots();
                    if n == 0 {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    buf.clear();
                    if let Ok(chunk) = ring.read_chunk(n) {
                        let (a, b) = chunk.as_slices();
                        buf.extend_from_slice(a);
                        buf.extend_from_slice(b);
                        chunk.commit_all();
                    }
                    let on = en.load(Ordering::Relaxed);
                    if !on {
                        if was_enabled {
                            live.reset();
                        }
                        was_enabled = false;
                        continue;
                    }
                    was_enabled = true;
                    let mut out = Vec::new();
                    live.push(&buf, |f| out.push(f));
                    if !out.is_empty() {
                        let mut q = fr.lock();
                        for f in out {
                            let s = sq.fetch_add(1, Ordering::Relaxed) + 1;
                            if q.len() >= KEEP {
                                q.pop_front();
                            }
                            q.push_back(LiveFrame { seq: s, frame: f });
                        }
                    }
                }
            })
            .expect("spawn live pitch thread");
        Self { enabled, frames, stop, handle: Some(handle), seq }
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
        if !on {
            self.frames.lock().clear();
        }
    }

    /// Frames newer than `since` (sequence number).
    pub fn frames_since(&self, since: u64, max: usize) -> Vec<LiveFrame> {
        let q = self.frames.lock();
        let mut v: Vec<LiveFrame> = q.iter().filter(|f| f.seq > since).copied().collect();
        if v.len() > max {
            v.drain(..v.len() - max);
        }
        v
    }

    pub fn latest_seq(&self) -> u64 {
        self.seq.load(Ordering::Relaxed)
    }
}

impl Drop for LiveWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
