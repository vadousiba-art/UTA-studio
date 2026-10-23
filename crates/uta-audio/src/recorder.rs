//! Recording writer thread.
//!
//! The audio thread pushes the raw interleaved input block into a lock-free
//! ring while recording. This thread drains it and writes one 32-bit float
//! WAV per record target (mono or stereo channel pair). WAV headers are
//! rewritten about once per second, so a crash or power loss leaves a valid
//! file containing everything up to the last flush.

use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use rtrb::Consumer;
use serde::{Deserialize, Serialize};

use crate::wav::{BitDepth, WavFileWriter};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordTarget {
    /// Track this take belongs to (0 for analyzer/coach recordings).
    pub track_id: u128,
    /// First input channel (0-based).
    pub channel: u16,
    pub stereo: bool,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedFile {
    pub track_id: u128,
    pub path: PathBuf,
    pub channels: u16,
    pub frames: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingResult {
    pub files: Vec<RecordedFile>,
    /// Frames the audio thread could not queue (ring full); 0 when healthy.
    pub dropped_frames: u64,
    /// Frames the writer did not receive before timing out at stop.
    pub missing_frames: u64,
    pub errors: Vec<String>,
}

enum Msg {
    Start {
        targets: Vec<RecordTarget>,
        sample_rate: u32,
        reply: Sender<Result<(), String>>,
    },
    Stop {
        total_frames: u64,
        dropped: u64,
        reply: Sender<RecordingResult>,
    },
    Shutdown,
}

struct Writer {
    target: RecordTarget,
    wav: WavFileWriter,
}

pub struct Recorder {
    tx: Sender<Msg>,
    handle: Option<JoinHandle<()>>,
}

impl Recorder {
    /// `ring` carries interleaved input with `input_channels` channels.
    pub fn spawn(ring: Consumer<f32>, input_channels: u16) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let handle = std::thread::Builder::new()
            .name("uta-recorder".into())
            .spawn(move || run(rx, ring, input_channels.max(1) as usize))
            .expect("spawn recorder thread");
        Self { tx, handle: Some(handle) }
    }

    /// Create the files. Must complete before the audio thread starts pushing.
    pub fn start(&self, targets: Vec<RecordTarget>, sample_rate: u32) -> Result<(), String> {
        let (reply, rx) = crossbeam_channel::bounded(1);
        self.tx
            .send(Msg::Start { targets, sample_rate, reply })
            .map_err(|_| "recorder thread is not running".to_string())?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "recorder did not respond".to_string())?
    }

    /// Finish writing `total_frames` frames (as counted by the audio thread)
    /// and close the files.
    pub fn stop(&self, total_frames: u64, dropped: u64) -> RecordingResult {
        let (reply, rx) = crossbeam_channel::bounded(1);
        if self.tx.send(Msg::Stop { total_frames, dropped, reply }).is_err() {
            return RecordingResult {
                files: vec![],
                dropped_frames: dropped,
                missing_frames: total_frames,
                errors: vec!["recorder thread is not running".into()],
            };
        }
        rx.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| RecordingResult {
            files: vec![],
            dropped_frames: dropped,
            missing_frames: total_frames,
            errors: vec!["recorder did not finish in time".into()],
        })
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn drain(ring: &mut Consumer<f32>, ch: usize, writers: &mut [Writer], scratch: &mut Vec<f32>, errors: &mut Vec<String>, max_frames: u64) -> u64 {
    let avail_frames = (ring.slots() / ch) as u64;
    let frames = avail_frames.min(max_frames) as usize;
    if frames == 0 {
        return 0;
    }
    let chunk = match ring.read_chunk(frames * ch) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let (a, b) = chunk.as_slices();
    for w in writers.iter_mut() {
        scratch.clear();
        let c0 = (w.target.channel as usize).min(ch - 1);
        let c1 = if w.target.stereo { (c0 + 1).min(ch - 1) } else { c0 };
        for f in 0..frames {
            let get = |i: usize| if i < a.len() { a[i] } else { b[i - a.len()] };
            scratch.push(get(f * ch + c0));
            if w.target.stereo {
                scratch.push(get(f * ch + c1));
            }
        }
        if let Err(e) = w.wav.write(scratch) {
            errors.push(format!("{}: {e}", w.target.path.display()));
        }
    }
    chunk.commit_all();
    frames as u64
}

fn run(rx: Receiver<Msg>, mut ring: Consumer<f32>, ch: usize) {
    let mut writers: Vec<Writer> = Vec::new();
    let mut written: u64 = 0;
    let mut errors: Vec<String> = Vec::new();
    let mut scratch = Vec::with_capacity(8192);
    let mut last_flush = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Msg::Start { targets, sample_rate, reply }) => {
                // discard stale data from a previous session
                let stale = ring.slots();
                if let Ok(c) = ring.read_chunk(stale) {
                    c.commit_all();
                }
                writers.clear();
                errors.clear();
                written = 0;
                let mut result = Ok(());
                for t in targets {
                    let channels = if t.stereo { 2 } else { 1 };
                    match WavFileWriter::create(&t.path, sample_rate, channels, BitDepth::Float32, false) {
                        Ok(wav) => writers.push(Writer { target: t, wav }),
                        Err(e) => {
                            result = Err(e.to_string());
                            writers.clear();
                            break;
                        }
                    }
                }
                let _ = reply.send(result);
            }
            Ok(Msg::Stop { total_frames, dropped, reply }) => {
                let deadline = Instant::now() + Duration::from_secs(3);
                while written < total_frames && Instant::now() < deadline {
                    let n = drain(&mut ring, ch, &mut writers, &mut scratch, &mut errors, total_frames - written);
                    written += n;
                    if n == 0 {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
                let missing = total_frames.saturating_sub(written);
                let mut files = Vec::new();
                for w in writers.drain(..) {
                    let channels = if w.target.stereo { 2 } else { 1 };
                    let path = w.target.path.clone();
                    let track_id = w.target.track_id;
                    match w.wav.finalize() {
                        Ok(frames) => files.push(RecordedFile { track_id, path, channels, frames }),
                        Err(e) => errors.push(format!("{}: {e}", path.display())),
                    }
                }
                let _ = reply.send(RecordingResult {
                    files,
                    dropped_frames: dropped,
                    missing_frames: missing,
                    errors: std::mem::take(&mut errors),
                });
                written = 0;
            }
            Ok(Msg::Shutdown) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                // Finalise whatever is open so files stay valid.
                for w in writers.drain(..) {
                    let _ = w.wav.finalize();
                }
                return;
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
        if !writers.is_empty() {
            written += drain(&mut ring, ch, &mut writers, &mut scratch, &mut errors, u64::MAX);
            if last_flush.elapsed() > Duration::from_secs(1) {
                for w in writers.iter_mut() {
                    if let Err(e) = w.wav.flush() {
                        errors.push(format!("{}: {e}", w.target.path.display()));
                    }
                }
                last_flush = Instant::now();
            }
        } else {
            // not recording: keep the ring empty
            let n = ring.slots();
            if n > 0 {
                if let Ok(c) = ring.read_chunk(n) {
                    c.commit_all();
                }
            }
        }
    }
}
