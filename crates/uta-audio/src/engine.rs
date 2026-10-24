//! Control-side engine API. Owns the backend (device streams or the null
//! driver), the streamer and recorder threads and the command ring into the
//! real-time core. Not thread-safe by itself: callers wrap it in a mutex.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};

use crate::core::{CaptureStart, Command, CoreConfig, EngineCore, Garbage, InputSource, MetronomeSettings, SeqNote};
use crate::device::{start_cpal, BackendInfo, CpalStreams, DeviceSelection};
use crate::error::{AudioError, Result};
use crate::graph::Graph;
use crate::recorder::{RecordTarget, Recorder, RecordingResult};
use crate::shared::{MeterValue, SharedState, MAX_INPUT_CHANNELS};
use crate::source::SourceSet;
use crate::streamer::Streamer;

/// Synthetic input for the null backend (tests and demos without hardware).
pub type InputGenerator = Box<dyn FnMut(&mut [f32], usize) + Send>;
/// Receives interleaved stereo output blocks from the null backend.
pub type OutputSink = Box<dyn FnMut(&[f32]) + Send>;

pub struct NullOptions {
    pub sample_rate: u32,
    pub input_channels: u16,
    pub block: usize,
    /// Fill interleaved input blocks; `None` = silence.
    pub input: Option<InputGenerator>,
    /// Run as fast as possible instead of in real time (tests).
    pub free_run: bool,
    /// Receives every interleaved stereo output block (tests, diagnostics).
    pub output_sink: Option<OutputSink>,
}

impl NullOptions {
    pub fn new(sample_rate: u32) -> Self {
        Self { sample_rate, input_channels: 0, block: 256, input: None, free_run: false, output_sink: None }
    }
}

pub enum BackendChoice {
    Device(DeviceSelection),
    Null(NullOptions),
}

struct NullDriver {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for NullDriver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

enum Backend {
    Device(#[allow(dead_code)] CpalStreams),
    Null(#[allow(dead_code)] NullDriver),
}

/// Live status snapshot for the UI.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub running: bool,
    pub backend: BackendInfo,
    pub playing: bool,
    pub recording: bool,
    pub position: u64,
    pub sample_rate: u32,
    pub record_frames: u64,
    pub preview_playing: bool,
    pub preview_position: u64,
    pub dsp_load: f32,
    pub input_underruns: u64,
    pub input_overflows: u64,
    pub stream_underruns: u64,
    pub device_xruns: u64,
    pub device_lost: bool,
    pub input_latency_ms: f32,
    pub output_latency_ms: f32,
    pub master: [MeterValue; 2],
    pub tracks: Vec<[MeterValue; 2]>,
    pub inputs: Vec<MeterValue>,
}

/// Result of a finished recording pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingOutcome {
    pub result: RecordingResult,
    /// Timeline frame the transport was at when capture began.
    pub start_position: u64,
    /// Latency compensation applied when placing clips (frames).
    pub latency_frames: u64,
    pub sample_rate: u32,
}

pub struct AudioEngine {
    shared: Arc<SharedState>,
    commands: Producer<Command>,
    garbage: Consumer<Garbage>,
    streamer: Streamer,
    recorder: Option<Recorder>,
    analysis: Option<Consumer<f32>>,
    backend: Backend,
    info: BackendInfo,
    generation: u64,
    loop_region: Option<(u64, u64)>,
    recording: bool,
    capture_only: bool,
    /// Extra latency compensation from the user's calibration (frames).
    pub manual_latency_frames: i64,
    /// Processing latency of the current graph (plugin bridge, PDC).
    graph_latency: u64,
    preview_generation: u64,
    track_count: usize,
}

const COMMAND_CAPACITY: usize = 1024;
const GARBAGE_CAPACITY: usize = 256;
const RECORD_SECONDS: usize = 4;

impl AudioEngine {
    pub fn start(choice: BackendChoice) -> Result<Self> {
        let shared = Arc::new(SharedState::new());
        let (cmd_tx, cmd_rx) = RingBuffer::<Command>::new(COMMAND_CAPACITY);
        let (garbage_tx, garbage_rx) = RingBuffer::<Garbage>::new(GARBAGE_CAPACITY);

        match choice {
            BackendChoice::Device(sel) => {
                let mut parts: Option<(Recorder, Consumer<f32>)> = None;
                let shared_c = shared.clone();
                let parts_ref = &mut parts;
                let factory = Box::new(move |cfg: CoreConfig, input_ring: Option<Consumer<f32>>| {
                    let ch = cfg.input_channels.max(1) as usize;
                    let (rec_tx, rec_rx) = RingBuffer::<f32>::new(cfg.sample_rate as usize * RECORD_SECONDS * ch);
                    let (an_tx, an_rx) = RingBuffer::<f32>::new(cfg.sample_rate as usize);
                    *parts_ref = Some((Recorder::spawn(rec_rx, cfg.input_channels), an_rx));
                    EngineCore::new(cfg, shared_c, cmd_rx, garbage_tx, input_ring, Some(rec_tx), Some(an_tx))
                });
                let (streams, info) = start_cpal(&sel, shared.clone(), factory)?;
                let (recorder, analysis) = parts.ok_or_else(|| AudioError::Other("engine core was not created".into()))?;
                let streamer = Streamer::spawn(info.sample_rate);
                Ok(Self::assemble(shared, cmd_tx, garbage_rx, streamer, Some(recorder), Some(analysis), Backend::Device(streams), info))
            }
            BackendChoice::Null(opts) => {
                let ch = (opts.input_channels as usize).min(MAX_INPUT_CHANNELS);
                let (rec_tx, rec_rx) = RingBuffer::<f32>::new(opts.sample_rate as usize * RECORD_SECONDS * ch.max(1));
                let (an_tx, an_rx) = RingBuffer::<f32>::new(opts.sample_rate as usize);
                let recorder = Recorder::spawn(rec_rx, ch as u16);
                let core = EngineCore::new(
                    CoreConfig { sample_rate: opts.sample_rate, input_channels: ch as u16 },
                    shared.clone(),
                    cmd_rx,
                    garbage_tx,
                    None,
                    Some(rec_tx),
                    Some(an_tx),
                );
                let info = BackendInfo {
                    kind: "null".into(),
                    host: "none".into(),
                    output_name: None,
                    input_name: if ch > 0 { Some("Synthetic input".into()) } else { None },
                    sample_rate: opts.sample_rate,
                    output_channels: 2,
                    input_channels: ch as u16,
                    buffer_frames: Some(opts.block as u32),
                    notes: vec!["No audio device is open; audio is processed silently.".into()],
                };
                let driver = spawn_null(core, opts);
                let streamer = Streamer::spawn(info.sample_rate);
                Ok(Self::assemble(shared, cmd_tx, garbage_rx, streamer, Some(recorder), Some(an_rx), Backend::Null(driver), info))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn assemble(
        shared: Arc<SharedState>,
        commands: Producer<Command>,
        garbage: Consumer<Garbage>,
        streamer: Streamer,
        recorder: Option<Recorder>,
        analysis: Option<Consumer<f32>>,
        backend: Backend,
        info: BackendInfo,
    ) -> Self {
        Self {
            shared,
            commands,
            garbage,
            streamer,
            recorder,
            analysis,
            backend,
            info,
            generation: 0,
            loop_region: None,
            recording: false,
            capture_only: false,
            manual_latency_frames: 0,
            graph_latency: 0,
            preview_generation: 0,
            track_count: 0,
        }
    }

    pub fn info(&self) -> &BackendInfo {
        &self.info
    }

    pub fn sample_rate(&self) -> u32 {
        self.info.sample_rate
    }

    pub fn shared(&self) -> &Arc<SharedState> {
        &self.shared
    }

    pub fn has_input(&self) -> bool {
        self.info.input_channels > 0
    }

    pub fn is_null(&self) -> bool {
        matches!(self.backend, Backend::Null(_))
    }

    /// Take the live-analysis sample stream (once).
    pub fn take_analysis_stream(&mut self) -> Option<Consumer<f32>> {
        self.analysis.take()
    }

    /// Drop objects returned by the audio thread.
    pub fn collect_garbage(&mut self) {
        while let Ok(g) = self.garbage.pop() {
            drop(g);
        }
    }

    pub fn send(&mut self, cmd: Command) -> Result<()> {
        self.collect_garbage();
        self.commands
            .push(cmd)
            .map_err(|_| AudioError::Busy("command queue is full".into()))
    }

    pub fn set_graph(&mut self, graph: Graph) -> Result<()> {
        self.track_count = graph.tracks.len();
        self.graph_latency = graph.latency() as u64;
        self.send(Command::SetGraph(Box::new(graph)))
    }

    pub fn set_sources(&mut self, set: SourceSet) {
        self.streamer.send(crate::streamer::StreamerMsg::SetSources(Arc::new(set)));
    }

    pub fn position(&self) -> u64 {
        self.shared.position.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    pub fn is_recording(&self) -> bool {
        self.recording
    }

    fn check_device(&self) -> Result<()> {
        if self.shared.device_lost.load(Ordering::Acquire) {
            return Err(AudioError::Disconnected(
                "the audio device stopped responding; choose a device in Settings to reconnect".into(),
            ));
        }
        Ok(())
    }

    /// Start playback from `from` (frames).
    pub fn play(&mut self, from: u64) -> Result<()> {
        self.start_transport(from, None)
    }

    fn start_transport(&mut self, from: u64, record: Option<CaptureStart>) -> Result<()> {
        self.check_device()?;
        self.generation += 1;
        let bundle = self
            .streamer
            .start(self.generation, from, self.loop_region)
            .ok_or_else(|| AudioError::Other("the streaming thread did not respond".into()))?;
        self.send(Command::Play {
            generation: bundle.generation,
            position: bundle.position,
            loop_region: self.loop_region,
            streams: Box::new(bundle.streams),
            record,
        })
    }

    pub fn stop(&mut self) -> Result<()> {
        self.streamer.send(crate::streamer::StreamerMsg::Stop);
        self.send(Command::Stop)
    }

    pub fn locate(&mut self, pos: u64) -> Result<()> {
        if self.is_playing() {
            self.play(pos)
        } else {
            self.send(Command::Locate(pos))
        }
    }

    /// Set or clear the loop. Restarts streams if playing.
    pub fn set_loop(&mut self, region: Option<(u64, u64)>) -> Result<()> {
        self.loop_region = region.filter(|(s, e)| e > s);
        if self.is_playing() && !self.recording {
            let pos = self.position();
            self.play(pos)?;
        }
        Ok(())
    }

    pub fn loop_region(&self) -> Option<(u64, u64)> {
        self.loop_region
    }

    /// Re-stream from the current position after the project's audio changed
    /// (clips moved, etc.). No-op when stopped.
    pub fn refresh_playback(&mut self) -> Result<()> {
        if self.is_playing() && !self.recording {
            let pos = self.position();
            self.play(pos)?;
        }
        Ok(())
    }

    /// Total latency compensation for recordings, in frames.
    pub fn latency_compensation(&self) -> u64 {
        let sr = self.info.sample_rate as f64;
        let measured_us = self.shared.input_latency_us.load(Ordering::Relaxed) + self.shared.output_latency_us.load(Ordering::Relaxed);
        let buffer = self.info.buffer_frames.unwrap_or(0) as i64;
        // the performer hears playback delayed by plugin latency as well
        let base = (measured_us as f64 * 1e-6 * sr) as i64 + buffer + self.graph_latency as i64;
        (base + self.manual_latency_frames).max(0) as u64
    }

    fn recorder(&self) -> Result<&Recorder> {
        if !self.has_input() {
            return Err(AudioError::NoDevice("no microphone is open; enable the input device in Settings".into()));
        }
        self.recorder.as_ref().ok_or(AudioError::NotRunning)
    }

    /// Studio recording: create files for `targets`, start the transport at
    /// `from` and begin capturing on the same audio block.
    pub fn record(&mut self, targets: Vec<RecordTarget>, from: u64) -> Result<()> {
        if self.recording {
            return Err(AudioError::Busy("already recording".into()));
        }
        if targets.is_empty() {
            return Err(AudioError::Other("arm at least one track to record".into()));
        }
        let sr = self.sample_rate();
        self.recorder()?.start(targets, sr).map_err(AudioError::Other)?;
        self.recording = true;
        self.capture_only = false;
        if let Err(e) = self.start_transport(from, Some(CaptureStart { with_transport: true })) {
            self.recording = false;
            let _ = self.recorder()?.stop(0, 0);
            return Err(e);
        }
        Ok(())
    }

    /// Analyzer/coach capture: record without moving the transport.
    /// `sequence` optionally plays a guide melody starting on the same frame.
    pub fn capture(&mut self, target: PathBuf, channel: u16, sequence: Option<(Vec<SeqNote>, Option<f32>)>) -> Result<()> {
        if self.recording {
            return Err(AudioError::Busy("already recording".into()));
        }
        self.check_device()?;
        let sr = self.sample_rate();
        self.recorder()?
            .start(vec![RecordTarget { track_id: 0, channel, stereo: false, path: target }], sr)
            .map_err(AudioError::Other)?;
        self.recording = true;
        self.capture_only = true;
        let r = match sequence {
            Some((notes, click)) => self.send(Command::Sequence {
                notes: Box::new(notes),
                with_capture: true,
                click_bpm: click,
            }),
            None => self.send(Command::StartCapture),
        };
        if r.is_err() {
            self.recording = false;
            let _ = self.recorder()?.stop(0, 0);
        }
        r
    }

    /// Stop recording/capture and close the files.
    pub fn stop_recording(&mut self) -> Result<RecordingOutcome> {
        if !self.recording {
            return Err(AudioError::Other("not recording".into()));
        }
        let was_capture = self.capture_only;
        if was_capture {
            self.send(Command::StopCapture)?;
            let _ = self.send(Command::StopSequence);
        } else {
            self.streamer.send(crate::streamer::StreamerMsg::Stop);
            self.send(Command::Stop)?;
        }
        // wait (briefly) for the audio thread to acknowledge
        let deadline = Instant::now() + Duration::from_millis(1500);
        while self.shared.recording.load(Ordering::Acquire) && Instant::now() < deadline {
            if self.shared.device_lost.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let total = self.shared.record_frames.load(Ordering::Acquire);
        let dropped = self.shared.record_dropped.load(Ordering::Relaxed);
        let result = self.recorder.as_ref().ok_or(AudioError::NotRunning)?.stop(total, dropped);
        self.recording = false;
        Ok(RecordingOutcome {
            result,
            start_position: self.shared.record_start.load(Ordering::Relaxed),
            latency_frames: if was_capture { 0 } else { self.latency_compensation() },
            sample_rate: self.sample_rate(),
        })
    }

    pub fn play_tone(&mut self, hz: f32, amp: f32, seconds: f32) -> Result<()> {
        let frames = (seconds.max(0.0) * self.sample_rate() as f32) as u64;
        self.send(Command::Tone { hz, amp, frames })
    }

    pub fn stop_tone(&mut self) -> Result<()> {
        self.send(Command::ToneOff)
    }

    pub fn play_sequence(&mut self, notes: Vec<SeqNote>, click_bpm: Option<f32>) -> Result<()> {
        self.send(Command::Sequence { notes: Box::new(notes), with_capture: false, click_bpm })
    }

    pub fn stop_sequence(&mut self) -> Result<()> {
        self.send(Command::StopSequence)
    }

    pub fn set_metronome(&mut self, m: MetronomeSettings) -> Result<()> {
        self.send(Command::Metronome(m))
    }

    /// Play a WAV file outside the timeline (take replay, reference audio).
    pub fn preview(&mut self, path: PathBuf, start_frame: u64, gain: f32) -> Result<u64> {
        self.preview_range(path, start_frame, None, false, gain)
    }

    /// Play `path` from `start_frame` to `end` (or the file end). With
    /// `looped`, playback wraps seamlessly from `end` back to `start_frame`
    /// until stopped. Returns the end frame.
    pub fn preview_range(&mut self, path: PathBuf, start_frame: u64, end: Option<u64>, looped: bool, gain: f32) -> Result<u64> {
        self.preview_generation += 1;
        let loop_start = looped.then_some(start_frame);
        let b = self
            .streamer
            .preview(self.preview_generation, path, start_frame, end, loop_start)
            .map_err(AudioError::Other)?;
        let frames = b.frames;
        self.send(Command::Preview {
            generation: b.generation,
            consumer: b.consumer,
            frames: b.frames,
            start: start_frame,
            gain,
            loop_start: loop_start.filter(|s| *s < frames),
        })?;
        Ok(frames)
    }

    pub fn stop_preview(&mut self) -> Result<()> {
        self.streamer.send(crate::streamer::StreamerMsg::StopPreview);
        self.send(Command::StopPreview)
    }

    pub fn set_direct_monitor(&mut self, channel: Option<u16>, gain: f32) -> Result<()> {
        self.send(Command::DirectMonitor { channel, gain })
    }

    pub fn set_analysis_channel(&mut self, channel: u16) -> Result<()> {
        self.send(Command::AnalysisChannel(channel))
    }

    pub fn reset_clip_indicators(&self) {
        for m in &self.shared.track_meters {
            m.reset_clip();
        }
        self.shared.master_meter.reset_clip();
        for m in &self.shared.input_meters {
            m.reset_clip();
        }
    }

    pub fn status(&mut self) -> EngineStatus {
        self.collect_garbage();
        let s = &self.shared;
        let n_tracks = self.track_count.min(s.track_meters.len());
        EngineStatus {
            running: true,
            backend: self.info.clone(),
            playing: s.playing.load(Ordering::Relaxed),
            recording: self.recording,
            position: s.position.load(Ordering::Relaxed),
            sample_rate: self.info.sample_rate,
            record_frames: s.record_frames.load(Ordering::Relaxed),
            preview_playing: s.preview_playing.load(Ordering::Relaxed),
            preview_position: s.preview_position.load(Ordering::Relaxed),
            dsp_load: s.dsp_load(),
            input_underruns: s.input_underruns.load(Ordering::Relaxed),
            input_overflows: s.input_overflows.load(Ordering::Relaxed),
            stream_underruns: s.stream_underruns.load(Ordering::Relaxed),
            device_xruns: s.device_xruns.load(Ordering::Relaxed),
            device_lost: s.device_lost.load(Ordering::Relaxed),
            input_latency_ms: s.input_latency_us.load(Ordering::Relaxed) as f32 / 1000.0,
            output_latency_ms: s.output_latency_us.load(Ordering::Relaxed) as f32 / 1000.0,
            master: s.master_meter.take(),
            tracks: s.track_meters[..n_tracks].iter().map(|m| m.take()).collect(),
            inputs: s.input_meters[..(self.info.input_channels as usize).min(MAX_INPUT_CHANNELS)]
                .iter()
                .map(|m| m.take())
                .collect(),
        }
    }
}

fn spawn_null(mut core: EngineCore, mut opts: NullOptions) -> NullDriver {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_c = stop.clone();
    let handle = std::thread::Builder::new()
        .name("uta-null-audio".into())
        .spawn(move || {
            let block = opts.block.clamp(16, 4096);
            let ch = opts.input_channels as usize;
            let mut out = vec![0.0f32; block * 2];
            let mut input = vec![0.0f32; block * ch.max(1)];
            let period = Duration::from_secs_f64(block as f64 / opts.sample_rate as f64);
            let mut next = Instant::now();
            while !stop_c.load(Ordering::Acquire) {
                let src = match opts.input.as_mut() {
                    Some(gen) if ch > 0 => {
                        gen(&mut input[..block * ch], ch);
                        InputSource::Slice(&input[..block * ch])
                    }
                    _ => InputSource::None,
                };
                core.process(src, &mut out, 2);
                if let Some(sink) = opts.output_sink.as_mut() {
                    sink(&out);
                }
                if opts.free_run {
                    // still yield so control threads make progress
                    std::thread::sleep(Duration::from_micros(200));
                } else {
                    next += period;
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else {
                        next = now;
                    }
                }
            }
        })
        .expect("spawn null audio thread");
    NullDriver { stop, handle: Some(handle) }
}
