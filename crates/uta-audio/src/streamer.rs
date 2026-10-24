//! Disk streaming thread.
//!
//! For every playback "generation" (each play/seek/loop change) the streamer
//! creates fresh lock-free ring buffers per track, pre-fills them from disk,
//! and hands the consumers to the audio thread. It then keeps them topped up.
//! Track audio is stored interleaved stereo in the rings. The audio thread
//! never touches files.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::source::{SourceReader, SourceSet};
use crate::wav::WavStreamReader;

/// Seconds of audio buffered per track.
pub const RING_SECONDS: f32 = 2.0;
/// Seconds pre-filled before playback starts.
pub const PREFILL_SECONDS: f32 = 0.4;
const CHUNK: usize = 2048;

pub struct StreamBundle {
    pub generation: u64,
    pub position: u64,
    pub streams: Vec<(u128, Consumer<f32>)>,
}

pub struct PreviewBundle {
    pub generation: u64,
    pub consumer: Consumer<f32>,
    pub frames: u64,
}

pub enum StreamerMsg {
    SetSources(Arc<SourceSet>),
    Start {
        generation: u64,
        position: u64,
        loop_region: Option<(u64, u64)>,
        reply: Sender<StreamBundle>,
    },
    Stop,
    Preview {
        generation: u64,
        path: std::path::PathBuf,
        start: u64,
        reply: Sender<Result<PreviewBundle, String>>,
    },
    StopPreview,
    Shutdown,
}

struct Active {
    producer: Producer<f32>,
    index: usize,
    pos: u64,
}

struct PreviewState {
    producer: Producer<f32>,
    reader: WavStreamReader,
    pos: u64,
}

pub struct Streamer {
    tx: Sender<StreamerMsg>,
    handle: Option<JoinHandle<()>>,
}

impl Streamer {
    pub fn spawn(sample_rate: u32) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let handle = std::thread::Builder::new()
            .name("uta-streamer".into())
            .spawn(move || run(rx, sample_rate))
            .expect("spawn streamer thread");
        Self { tx, handle: Some(handle) }
    }

    pub fn send(&self, msg: StreamerMsg) {
        // The thread only exits on Shutdown, so a send error means it panicked;
        // playback then simply stays silent and the error is logged.
        if self.tx.send(msg).is_err() {
            tracing::error!("streamer thread is not running");
        }
    }

    /// Start a generation and wait for the pre-filled streams.
    pub fn start(&self, generation: u64, position: u64, loop_region: Option<(u64, u64)>) -> Option<StreamBundle> {
        let (reply, rx) = crossbeam_channel::bounded(1);
        self.send(StreamerMsg::Start { generation, position, loop_region, reply });
        rx.recv_timeout(Duration::from_secs(5)).ok()
    }

    pub fn preview(&self, generation: u64, path: std::path::PathBuf, start: u64) -> Result<PreviewBundle, String> {
        let (reply, rx) = crossbeam_channel::bounded(1);
        self.send(StreamerMsg::Preview { generation, path, start, reply });
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "streamer did not respond".to_string())?
    }
}

impl Drop for Streamer {
    fn drop(&mut self) {
        let _ = self.tx.send(StreamerMsg::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn fill(active: &mut Active, reader: &mut SourceReader, loop_region: Option<(u64, u64)>, l: &mut [f32], r: &mut [f32], max_frames: usize) {
    let mut remaining = (active.producer.slots() / 2).min(max_frames);
    while remaining > 0 {
        let mut n = remaining.min(CHUNK);
        if let Some((_, le)) = loop_region {
            if active.pos < le {
                n = n.min((le - active.pos) as usize);
            }
        }
        if n == 0 {
            break;
        }
        reader.render(active.index, active.pos, &mut l[..n], &mut r[..n]);
        if let Ok(mut chunk) = active.producer.write_chunk_uninit(n * 2) {
            let (a, b) = chunk.as_mut_slices();
            for (k, slot) in a.iter_mut().chain(b.iter_mut()).enumerate() {
                let frame = k / 2;
                slot.write(if k % 2 == 0 { l[frame] } else { r[frame] });
            }
            // SAFETY: all n*2 slots were initialised above.
            unsafe { chunk.commit_all() };
        } else {
            break;
        }
        active.pos += n as u64;
        if let Some((ls, le)) = loop_region {
            if active.pos == le {
                active.pos = ls;
            }
        }
        remaining -= n;
    }
}

fn fill_preview(p: &mut PreviewState, tmp: &mut Vec<f32>) {
    let mut remaining = p.producer.slots() / 2;
    while remaining > 0 {
        let n = remaining.min(CHUNK);
        let got = p.reader.read(p.pos, n, tmp).unwrap_or_default();
        let ch = p.reader.info.channels as usize;
        let (frames, eof) = if got == 0 { (n, true) } else { (got, false) };
        if let Ok(mut chunk) = p.producer.write_chunk_uninit(frames * 2) {
            let (a, b) = chunk.as_mut_slices();
            for (k, slot) in a.iter_mut().chain(b.iter_mut()).enumerate() {
                let f = k / 2;
                let v = if eof {
                    0.0
                } else if ch == 1 {
                    tmp[f]
                } else {
                    tmp[f * ch + (k % 2)]
                };
                slot.write(v);
            }
            // SAFETY: every slot written above.
            unsafe { chunk.commit_all() };
        }
        if eof {
            break; // the audio thread stops at `frames`
        }
        p.pos += got as u64;
        remaining -= frames;
    }
}

fn run(rx: Receiver<StreamerMsg>, sample_rate: u32) {
    let ring_frames = (RING_SECONDS * sample_rate as f32) as usize;
    let prefill = (PREFILL_SECONDS * sample_rate as f32) as usize;
    let mut reader = SourceReader::new(Arc::new(SourceSet::default()));
    let mut active: Vec<Active> = Vec::new();
    let mut loop_region: Option<(u64, u64)> = None;
    let mut preview: Option<PreviewState> = None;
    let mut l = vec![0.0f32; CHUNK];
    let mut r = vec![0.0f32; CHUNK];
    let mut tmp = Vec::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(5)) {
            Ok(StreamerMsg::SetSources(set)) => {
                reader = SourceReader::new(set);
            }
            Ok(StreamerMsg::Start { generation, position, loop_region: lr, reply }) => {
                active.clear();
                loop_region = lr.filter(|(s, e)| e > s && position < *e);
                let mut streams = Vec::new();
                let set = reader.set().clone();
                for (index, t) in set.tracks.iter().enumerate() {
                    let (producer, consumer) = RingBuffer::<f32>::new(ring_frames * 2);
                    let mut a = Active { producer, index, pos: position };
                    fill(&mut a, &mut reader, loop_region, &mut l, &mut r, prefill);
                    active.push(a);
                    streams.push((t.id, consumer));
                }
                for (f, e) in reader.errors.drain(..) {
                    tracing::warn!("stream read error on file {f}: {e}");
                }
                let _ = reply.send(StreamBundle { generation, position, streams });
            }
            Ok(StreamerMsg::Stop) => active.clear(),
            Ok(StreamerMsg::Preview { generation, path, start, reply }) => {
                preview = None;
                match WavStreamReader::open(&path) {
                    Ok(rd) => {
                        let frames = rd.info.frames;
                        let (producer, consumer) = RingBuffer::<f32>::new(ring_frames * 2);
                        let mut p = PreviewState { producer, reader: rd, pos: start.min(frames) };
                        fill_preview(&mut p, &mut tmp);
                        preview = Some(p);
                        let _ = reply.send(Ok(PreviewBundle { generation, consumer, frames }));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e.to_string()));
                    }
                }
            }
            Ok(StreamerMsg::StopPreview) => preview = None,
            Ok(StreamerMsg::Shutdown) => return,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
        }
        for a in active.iter_mut() {
            if a.producer.is_abandoned() {
                continue;
            }
            fill(a, &mut reader, loop_region, &mut l, &mut r, usize::MAX);
        }
        active.retain(|a| !a.producer.is_abandoned());
        if let Some(p) = preview.as_mut() {
            if p.producer.is_abandoned() {
                preview = None;
            } else {
                fill_preview(p, &mut tmp);
            }
        }
    }
}
