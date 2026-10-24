//! Running a helper-hosted plugin inside the mixing graph.
//!
//! The audio thread must never wait for another process, so the real-time
//! endpoint ([`RtEndpoint`]) only touches lock-free ring buffers: it pushes
//! its input to a worker thread and pulls processed audio that the worker
//! got back from the helper. The output stream is primed with
//! [`BRIDGE_LATENCY`] frames, which is reported as latency so the graph's
//! delay compensation keeps every track aligned.
//!
//! When the helper is late, the endpoint substitutes the (equally delayed)
//! dry signal for the missing frames and later discards the same number of
//! late frames, so the plugin output never drifts out of alignment. When the
//! helper crashes or hangs, the slot keeps passing the delayed dry signal.
//!
//! Offline rendering uses [`OfflineProcessor`], which simply waits for each
//! block (no added latency beyond the plugin's own).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rtrb::{Consumer, Producer, RingBuffer};
use uta_audio::graph::{ChainProcessor, FixedDelay, MAX_BLOCK};

use crate::client::{HelperClient, AUDIO_TIMEOUT};

/// Frames of buffering between the audio thread and the helper. Large
/// enough for audio callbacks of up to 1024 frames plus the round trip.
pub const BRIDGE_LATENCY: usize = 1024;
const RING_FRAMES: usize = 16_384;

/// A live helper shared by the manager (control requests) and the
/// real-time worker (audio).
pub struct Link {
    pub client: Mutex<HelperClient>,
    failed: AtomicBool,
    failure: Mutex<Option<String>>,
    /// Blocks where the helper was late and the dry signal was used.
    pub late_blocks: AtomicU64,
    /// The plugin's own reported latency.
    pub plugin_latency: usize,
}

impl Link {
    pub fn new(client: HelperClient, plugin_latency: usize) -> Arc<Self> {
        Arc::new(Self {
            client: Mutex::new(client),
            failed: AtomicBool::new(false),
            failure: Mutex::new(None),
            late_blocks: AtomicU64::new(0),
            plugin_latency,
        })
    }

    pub fn is_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    pub fn failure(&self) -> Option<String> {
        self.failure.lock().clone()
    }

    pub fn mark_failed(&self, message: String) {
        tracing::error!("plugin failed: {message}");
        let mut f = self.failure.lock();
        if f.is_none() {
            *f = Some(message);
        }
        self.failed.store(true, Ordering::Release);
    }

    /// Run a control request, marking the link failed if the helper died.
    pub fn with_client<T>(&self, f: impl FnOnce(&mut HelperClient) -> Result<T, crate::ClientError>) -> Result<T, crate::ClientError> {
        let mut c = self.client.lock();
        let r = f(&mut c);
        if let Some(d) = c.death() {
            self.mark_failed(d.to_string());
        }
        r
    }
}

/// The real-time side of the bridge (lives in the audio graph).
pub struct RtEndpoint {
    link: Arc<Link>,
    to_worker: Producer<f32>,
    from_worker: Consumer<f32>,
    dry: FixedDelay,
    dry_l: Vec<f32>,
    dry_r: Vec<f32>,
    /// Late plugin frames still to be discarded to restore alignment.
    debt: usize,
    latency: usize,
    alive: Arc<AtomicBool>,
}

impl RtEndpoint {
    pub fn new(link: Arc<Link>) -> Self {
        let latency = BRIDGE_LATENCY + link.plugin_latency;
        let (to_worker, worker_in) = RingBuffer::<f32>::new(RING_FRAMES * 2);
        let (mut worker_out, from_worker) = RingBuffer::<f32>::new(RING_FRAMES * 2);
        // prime the output with silence: this is the bridge latency
        for _ in 0..BRIDGE_LATENCY * 2 {
            let _ = worker_out.push(0.0);
        }
        let alive = Arc::new(AtomicBool::new(true));
        let worker_link = link.clone();
        let worker_alive = alive.clone();
        let spawned = std::thread::Builder::new()
            .name("plugin-bridge".into())
            .spawn(move || worker(worker_link, worker_alive, worker_in, worker_out));
        if let Err(e) = spawned {
            link.mark_failed(format!("could not start the plugin worker thread: {e}"));
        }
        Self {
            link,
            to_worker,
            from_worker,
            dry: FixedDelay::new(latency),
            dry_l: vec![0.0; MAX_BLOCK],
            dry_r: vec![0.0; MAX_BLOCK],
            debt: 0,
            latency,
            alive,
        }
    }

    fn run(&mut self, left: &mut [f32], right: &mut [f32], bypassed: bool) {
        let n = left.len().min(MAX_BLOCK);
        let (left, right) = (&mut left[..n], &mut right[..n]);
        let (dl, dr) = (&mut self.dry_l[..n], &mut self.dry_r[..n]);
        dl.copy_from_slice(left);
        dr.copy_from_slice(right);
        self.dry.process(dl, dr);
        if self.link.is_failed() {
            left.copy_from_slice(dl);
            right.copy_from_slice(dr);
            return;
        }
        // send this block to the worker
        match self.to_worker.write_chunk_uninit(n * 2) {
            Ok(chunk) => {
                let it = left.iter().zip(right.iter()).flat_map(|(a, b)| [*a, *b]);
                chunk.fill_from_iter(it);
            }
            // worker hopelessly behind: this block will never come back
            Err(_) => self.debt += n,
        }
        // drop late frames owed from earlier blocks
        let avail = self.from_worker.slots() / 2;
        let d = self.debt.min(avail);
        if d > 0 {
            if let Ok(c) = self.from_worker.read_chunk(d * 2) {
                c.commit_all();
            }
            self.debt -= d;
        }
        let m = (self.from_worker.slots() / 2).min(n);
        if m > 0 {
            if let Ok(c) = self.from_worker.read_chunk(m * 2) {
                let (a, b) = c.as_slices();
                let mut it = a.iter().chain(b.iter());
                for k in 0..m {
                    let (pl, pr) = (*it.next().unwrap_or(&0.0), *it.next().unwrap_or(&0.0));
                    left[k] = pl;
                    right[k] = pr;
                }
                c.commit_all();
            }
        }
        if m < n {
            left[m..].copy_from_slice(&dl[m..]);
            right[m..].copy_from_slice(&dr[m..]);
            self.debt += n - m;
            self.link.late_blocks.fetch_add(1, Ordering::Relaxed);
        }
        if bypassed {
            left.copy_from_slice(dl);
            right.copy_from_slice(dr);
        }
    }
}

impl ChainProcessor for RtEndpoint {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.run(left, right, false);
    }
    /// Bypassed plugins keep running (so tails and state stay continuous)
    /// but the delayed dry signal is output.
    fn process_bypassed(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.run(left, right, true);
    }
    fn set_param(&mut self, _index: u32, _value: f32) {
        // plugin parameters travel over the control channel, not the graph
    }
    fn reset(&mut self) {
        self.dry.reset();
    }
    fn latency_samples(&self) -> usize {
        self.latency
    }
}

impl Drop for RtEndpoint {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

fn worker(link: Arc<Link>, alive: Arc<AtomicBool>, mut input: Consumer<f32>, mut output: Producer<f32>) {
    let mut l = vec![0.0f32; MAX_BLOCK];
    let mut r = vec![0.0f32; MAX_BLOCK];
    while alive.load(Ordering::Acquire) && !link.is_failed() {
        let avail = (input.slots() / 2).min(MAX_BLOCK);
        if avail == 0 {
            std::thread::sleep(Duration::from_micros(500));
            continue;
        }
        if let Ok(c) = input.read_chunk(avail * 2) {
            let (a, b) = c.as_slices();
            let mut it = a.iter().chain(b.iter());
            for k in 0..avail {
                l[k] = *it.next().unwrap_or(&0.0);
                r[k] = *it.next().unwrap_or(&0.0);
            }
            c.commit_all();
        }
        let res = link.with_client(|c| c.process(&[], &mut l[..avail], &mut r[..avail], AUDIO_TIMEOUT));
        if let Err(e) = res {
            link.mark_failed(e.to_string());
            break;
        }
        if let Ok(chunk) = output.write_chunk_uninit(avail * 2) {
            chunk.fill_from_iter(l[..avail].iter().zip(r[..avail].iter()).flat_map(|(a, b)| [*a, *b]));
        }
    }
}

/// Synchronous processor used for offline rendering. It owns its own
/// helper so exports never disturb live playback.
pub struct OfflineProcessor {
    client: HelperClient,
    name: String,
    latency: usize,
    dry: FixedDelay,
    dry_l: Vec<f32>,
    dry_r: Vec<f32>,
    errors: Arc<Mutex<Vec<String>>>,
    failed: bool,
}

impl OfflineProcessor {
    pub fn new(client: HelperClient, name: String, latency: usize, errors: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            client,
            name,
            latency,
            dry: FixedDelay::new(latency),
            dry_l: vec![0.0; MAX_BLOCK],
            dry_r: vec![0.0; MAX_BLOCK],
            errors,
            failed: false,
        }
    }

    fn run(&mut self, left: &mut [f32], right: &mut [f32], bypassed: bool) {
        let n = left.len().min(MAX_BLOCK);
        let (dl, dr) = (&mut self.dry_l[..n], &mut self.dry_r[..n]);
        dl.copy_from_slice(&left[..n]);
        dr.copy_from_slice(&right[..n]);
        self.dry.process(dl, dr);
        if !self.failed {
            if let Err(e) = self.client.process(&[], &mut left[..n], &mut right[..n], AUDIO_TIMEOUT * 5) {
                self.failed = true;
                self.errors
                    .lock()
                    .push(format!("{} stopped working during export ({e}); the rest of the export uses the unprocessed signal", self.name));
            }
        }
        if self.failed || bypassed {
            left[..n].copy_from_slice(dl);
            right[..n].copy_from_slice(dr);
        }
    }
}

impl ChainProcessor for OfflineProcessor {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.run(left, right, false);
    }
    fn process_bypassed(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.run(left, right, true);
    }
    fn set_param(&mut self, _index: u32, _value: f32) {}
    fn reset(&mut self) {
        self.dry.reset();
    }
    fn latency_samples(&self) -> usize {
        self.latency
    }
}
