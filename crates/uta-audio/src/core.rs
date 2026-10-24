//! The real-time engine core, owned by the audio output callback.
//!
//! Rules for everything reachable from [`EngineCore::process`]:
//! no allocation, no locks, no I/O, no blocking. Control threads talk to it
//! through a bounded lock-free command ring; objects it replaces are sent
//! back through a garbage ring to be dropped elsewhere.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use rtrb::{Consumer, Producer};
use uta_dsp::tone::{Click, ToneGenerator};

use crate::graph::{Graph, TrackFeed, MAX_BLOCK};
use crate::shared::{peak_rms, SharedState, MAX_INPUT_CHANNELS};

/// One note of a guide sequence, timed in frames from the sequence start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeqNote {
    pub start: u64,
    pub duration: u64,
    pub hz: f32,
    pub amp: f32,
}

/// Recording parameters handed to the core.
#[derive(Debug, Clone, Copy)]
pub struct CaptureStart {
    /// Start the transport too (studio recording) instead of only capturing.
    pub with_transport: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetronomeSettings {
    pub enabled: bool,
    pub bpm: f32,
    pub beats_per_bar: u32,
    pub gain: f32,
}

/// Boxed vectors keep `Command` small (it lives in a fixed-size ring).
#[allow(clippy::box_collection)]
pub enum Command {
    /// Start playback with freshly pre-filled streams.
    Play {
        generation: u64,
        position: u64,
        loop_region: Option<(u64, u64)>,
        streams: Box<Vec<(u128, Consumer<f32>)>>,
        record: Option<CaptureStart>,
    },
    Stop,
    /// Move the (stopped) transport.
    Locate(u64),
    SetGraph(Box<Graph>),
    SetTrackMix {
        id: u128,
        gain: f32,
        pan: (f32, f32),
        audible: bool,
        monitor: bool,
        input: crate::graph::InputRoute,
    },
    SetMasterGain(f32),
    /// `track == None` targets the master chain.
    SetEffectParam {
        track: Option<u128>,
        slot: u128,
        param: u32,
        value: f32,
    },
    SetEffectBypass {
        track: Option<u128>,
        slot: u128,
        bypass: bool,
    },
    /// Start capturing input without moving the transport (analyzer/coach).
    StartCapture,
    StopCapture,
    /// Play a reference tone.
    Tone { hz: f32, amp: f32, frames: u64 },
    ToneOff,
    /// Play a guide melody; if `with_capture` the capture starts on the same
    /// frame, so note times are exact relative to the recording.
    Sequence {
        notes: Box<Vec<SeqNote>>,
        with_capture: bool,
        click_bpm: Option<f32>,
    },
    StopSequence,
    Metronome(MetronomeSettings),
    Preview {
        generation: u64,
        consumer: Consumer<f32>,
        /// Frame where playback stops (or wraps when `loop_start` is set).
        frames: u64,
        start: u64,
        gain: f32,
        loop_start: Option<u64>,
    },
    StopPreview,
    /// Route one input channel straight to the output (analyzer monitoring).
    DirectMonitor { channel: Option<u16>, gain: f32 },
    /// Choose which input channel feeds the live analysis ring.
    AnalysisChannel(u16),
}

/// Objects the audio thread hands back for dropping on a normal thread.
#[allow(clippy::box_collection)]
pub enum Garbage {
    Graph(Box<Graph>),
    Streams(Box<Vec<(u128, Consumer<f32>)>>),
    Stream(Consumer<f32>),
    Sequence(Box<Vec<SeqNote>>),
}

struct StreamSlot {
    id: u128,
    consumer: Option<Consumer<f32>>,
    /// Frames owed after an underrun, discarded when data arrives to keep
    /// tracks in sync.
    debt: usize,
}

struct StreamFeed<'a> {
    slots: &'a mut [StreamSlot],
    underruns: u64,
}

impl TrackFeed for StreamFeed<'_> {
    fn fill(&mut self, _index: usize, id: u128, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        let Some(slot) = self.slots.iter_mut().find(|s| s.id == id) else {
            l.fill(0.0);
            r.fill(0.0);
            return;
        };
        let Some(c) = slot.consumer.as_mut() else {
            l.fill(0.0);
            r.fill(0.0);
            return;
        };
        // pay back debt from earlier underruns
        if slot.debt > 0 {
            let avail = c.slots() / 2;
            let pay = avail.min(slot.debt);
            if pay > 0 {
                if let Ok(ch) = c.read_chunk(pay * 2) {
                    ch.commit_all();
                }
                slot.debt -= pay;
            }
        }
        let avail = c.slots() / 2;
        let got = avail.min(n);
        if got > 0 {
            if let Ok(ch) = c.read_chunk(got * 2) {
                let (a, b) = ch.as_slices();
                for k in 0..got {
                    let i = 2 * k;
                    let get = |j: usize| if j < a.len() { a[j] } else { b[j - a.len()] };
                    l[k] = get(i);
                    r[k] = get(i + 1);
                }
                ch.commit_all();
            }
        }
        if got < n {
            l[got..].fill(0.0);
            r[got..].fill(0.0);
            slot.debt += n - got;
            self.underruns += 1;
        }
    }
}

struct Preview {
    consumer: Consumer<f32>,
    frames: u64,
    pos: u64,
    gain: f32,
    loop_start: Option<u64>,
}

struct Sequencer {
    // boxed so the swap through the command ring moves one pointer and the
    // old list is returned for deallocation off the audio thread
    #[allow(clippy::box_collection)]
    notes: Option<Box<Vec<SeqNote>>>,
    next: usize,
    frame: u64,
    click_period: Option<f64>,
    next_click: f64,
}

/// Where the input for a block comes from.
pub enum InputSource<'a> {
    /// No input device: silence.
    None,
    /// Interleaved samples supplied directly (tests, offline drivers).
    Slice(&'a [f32]),
    /// Pull from the input-callback ring.
    Ring,
}

pub struct EngineCore {
    sample_rate: f32,
    shared: Arc<SharedState>,
    commands: Consumer<Command>,
    garbage: Producer<Garbage>,
    graph: Box<Graph>,
    streams: Vec<StreamSlot>,
    playing: bool,
    position: u64,
    loop_region: Option<(u64, u64)>,
    input_channels: usize,
    input_ring: Option<Consumer<f32>>,
    inputs: Vec<Vec<f32>>,
    record_tx: Option<Producer<f32>>,
    capturing: bool,
    analysis_tx: Option<Producer<f32>>,
    analysis_channel: usize,
    tone: ToneGenerator,
    click: Click,
    metronome: MetronomeSettings,
    sequencer: Sequencer,
    preview: Option<Preview>,
    direct_monitor: Option<(usize, f32)>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    fx_l: Vec<f32>,
    /// Target number of frames queued in the input ring.
    input_target: usize,
}

pub struct CoreConfig {
    pub sample_rate: u32,
    pub input_channels: u16,
}

impl EngineCore {
    pub fn new(
        cfg: CoreConfig,
        shared: Arc<SharedState>,
        commands: Consumer<Command>,
        garbage: Producer<Garbage>,
        input_ring: Option<Consumer<f32>>,
        record_tx: Option<Producer<f32>>,
        analysis_tx: Option<Producer<f32>>,
    ) -> Self {
        let sr = cfg.sample_rate as f32;
        let ch = (cfg.input_channels as usize).min(MAX_INPUT_CHANNELS);
        shared.sample_rate.store(cfg.sample_rate, Ordering::Relaxed);
        Self {
            sample_rate: sr,
            shared,
            commands,
            garbage,
            graph: Box::new(Graph::new(sr)),
            streams: Vec::with_capacity(crate::shared::MAX_TRACKS),
            playing: false,
            position: 0,
            loop_region: None,
            input_channels: ch,
            input_ring,
            inputs: (0..ch).map(|_| vec![0.0; MAX_BLOCK]).collect(),
            record_tx,
            capturing: false,
            analysis_tx,
            analysis_channel: 0,
            tone: ToneGenerator::new(sr),
            click: Click::new(sr),
            metronome: MetronomeSettings { enabled: false, bpm: 120.0, beats_per_bar: 4, gain: 0.3 },
            sequencer: Sequencer { notes: None, next: 0, frame: 0, click_period: None, next_click: 0.0 },
            preview: None,
            direct_monitor: None,
            out_l: vec![0.0; MAX_BLOCK],
            out_r: vec![0.0; MAX_BLOCK],
            fx_l: vec![0.0; MAX_BLOCK],
            input_target: (sr * 0.02) as usize,
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    fn dispose(&mut self, g: Garbage) {
        // If the garbage ring is full, dropping here is the lesser evil.
        let _ = self.garbage.push(g);
    }

    fn handle_commands(&mut self) {
        while let Ok(cmd) = self.commands.pop() {
            match cmd {
                Command::Play { generation, position, loop_region, mut streams, record } => {
                    // swap in the new streams, keyed by track id
                    for s in self.streams.drain(..) {
                        if let Some(c) = s.consumer {
                            let _ = self.garbage.push(Garbage::Stream(c));
                        }
                    }
                    for (id, c) in streams.drain(..) {
                        self.streams.push(StreamSlot { id, consumer: Some(c), debt: 0 });
                    }
                    self.dispose(Garbage::Streams(streams));
                    self.position = position;
                    self.loop_region = loop_region.filter(|(s, e)| e > s && position < *e);
                    self.playing = true;
                    self.shared.stream_generation.store(generation, Ordering::Relaxed);
                    if record.map(|r| r.with_transport).unwrap_or(false) {
                        self.begin_capture();
                    }
                }
                Command::Stop => {
                    self.playing = false;
                    for s in self.streams.drain(..) {
                        if let Some(c) = s.consumer {
                            let _ = self.garbage.push(Garbage::Stream(c));
                        }
                    }
                    if self.capturing {
                        self.end_capture();
                    }
                    self.graph.reset();
                }
                Command::Locate(pos) => {
                    if !self.playing {
                        self.position = pos;
                    }
                }
                Command::SetGraph(g) => {
                    let old = std::mem::replace(&mut self.graph, g);
                    self.dispose(Garbage::Graph(old));
                }
                Command::SetTrackMix { id, gain, pan, audible, monitor, input } => {
                    if let Some(t) = self.graph.track_mut(id) {
                        t.set_gain(gain, false);
                        t.pan = pan;
                        t.audible = audible;
                        t.monitor = monitor;
                        t.input = input;
                    }
                }
                Command::SetMasterGain(g) => self.graph.master_gain.set_target(g),
                Command::SetEffectParam { track, slot, param, value } => {
                    let chain = match track {
                        Some(id) => self.graph.track_mut(id).map(|t| &mut t.chain),
                        None => Some(&mut self.graph.master),
                    };
                    if let Some(s) = chain.and_then(|c| c.slot_mut(slot)) {
                        s.processor.set_param(param, value);
                    }
                }
                Command::SetEffectBypass { track, slot, bypass } => {
                    let chain = match track {
                        Some(id) => self.graph.track_mut(id).map(|t| &mut t.chain),
                        None => Some(&mut self.graph.master),
                    };
                    if let Some(s) = chain.and_then(|c| c.slot_mut(slot)) {
                        s.bypass = bypass;
                        if bypass {
                            s.processor.reset();
                        }
                    }
                }
                Command::StartCapture => self.begin_capture(),
                Command::StopCapture => self.end_capture(),
                Command::Tone { hz, amp, frames } => self.tone.note_on(hz, amp, frames),
                Command::ToneOff => self.tone.note_off(),
                Command::Sequence { notes, with_capture, click_bpm } => {
                    if let Some(old) = self.sequencer.notes.take() {
                        self.dispose(Garbage::Sequence(old));
                    }
                    self.sequencer.notes = Some(notes);
                    self.sequencer.next = 0;
                    self.sequencer.frame = 0;
                    self.sequencer.click_period = click_bpm.map(|b| self.sample_rate as f64 * 60.0 / b.max(20.0) as f64);
                    self.sequencer.next_click = 0.0;
                    if with_capture {
                        self.begin_capture();
                    }
                }
                Command::StopSequence => {
                    if let Some(old) = self.sequencer.notes.take() {
                        self.dispose(Garbage::Sequence(old));
                    }
                    self.sequencer.click_period = None;
                    self.tone.note_off();
                }
                Command::Metronome(m) => self.metronome = m,
                Command::Preview { generation: _, consumer, frames, start, gain, loop_start } => {
                    if let Some(p) = self.preview.take() {
                        self.dispose(Garbage::Stream(p.consumer));
                    }
                    self.preview = Some(Preview { consumer, frames, pos: start, gain, loop_start });
                    let at = if self.capturing { self.shared.record_frames.load(Ordering::Relaxed) } else { u64::MAX };
                    self.shared.preview_start_record_frame.store(at, Ordering::Relaxed);
                    self.shared.preview_playing.store(true, Ordering::Relaxed);
                }
                Command::StopPreview => {
                    if let Some(p) = self.preview.take() {
                        self.dispose(Garbage::Stream(p.consumer));
                    }
                    self.shared.preview_playing.store(false, Ordering::Relaxed);
                }
                Command::DirectMonitor { channel, gain } => {
                    self.direct_monitor = channel.map(|c| (c as usize, gain));
                }
                Command::AnalysisChannel(c) => self.analysis_channel = c as usize,
            }
        }
    }

    fn begin_capture(&mut self) {
        if self.record_tx.is_none() || self.input_channels == 0 {
            return;
        }
        self.capturing = true;
        self.shared.record_start.store(self.position, Ordering::Relaxed);
        self.shared.record_frames.store(0, Ordering::Relaxed);
        self.shared.preview_start_record_frame.store(u64::MAX, Ordering::Relaxed);
        self.shared.record_dropped.store(0, Ordering::Relaxed);
        self.shared.recording.store(true, Ordering::Release);
    }

    fn end_capture(&mut self) {
        self.capturing = false;
        self.shared.recording.store(false, Ordering::Release);
    }

    /// Fill `self.inputs` for `frames` frames.
    fn read_input(&mut self, frames: usize, src: &InputSource, offset: usize) {
        let ch = self.input_channels;
        if ch == 0 {
            return;
        }
        match src {
            InputSource::None => {
                for c in &mut self.inputs {
                    c[..frames].fill(0.0);
                }
            }
            InputSource::Slice(data) => {
                let devch = ch;
                for f in 0..frames {
                    for c in 0..ch {
                        self.inputs[c][f] = data.get((offset + f) * devch + c).copied().unwrap_or(0.0);
                    }
                }
            }
            InputSource::Ring => {
                let Some(ring) = self.input_ring.as_mut() else {
                    for c in &mut self.inputs {
                        c[..frames].fill(0.0);
                    }
                    return;
                };
                // keep latency bounded: drop excess beyond target + block
                let queued = ring.slots() / ch;
                let limit = self.input_target + frames * 2;
                if queued > limit {
                    let drop = queued - (self.input_target + frames);
                    if let Ok(c) = ring.read_chunk(drop * ch) {
                        c.commit_all();
                    }
                    self.shared.input_overflows.fetch_add(1, Ordering::Relaxed);
                }
                let avail = (ring.slots() / ch).min(frames);
                if let Ok(chunk) = ring.read_chunk(avail * ch) {
                    let (a, b) = chunk.as_slices();
                    for f in 0..avail {
                        for c in 0..ch {
                            let i = f * ch + c;
                            self.inputs[c][f] = if i < a.len() { a[i] } else { b[i - a.len()] };
                        }
                    }
                    chunk.commit_all();
                }
                if avail < frames {
                    for c in &mut self.inputs {
                        c[avail..frames].fill(0.0);
                    }
                    if self.shared.callbacks.load(Ordering::Relaxed) > 4 {
                        self.shared.input_underruns.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }

    fn tap_input(&mut self, frames: usize) {
        let ch = self.input_channels;
        for c in 0..ch {
            let (p, r) = peak_rms(&self.inputs[c][..frames]);
            self.shared.input_meters[c].update(p, r);
        }
        if let Some(tx) = self.analysis_tx.as_mut() {
            let c = self.analysis_channel.min(ch.saturating_sub(1));
            if ch > 0 {
                let n = tx.slots().min(frames);
                if let Ok(mut chunk) = tx.write_chunk_uninit(n) {
                    let (a, b) = chunk.as_mut_slices();
                    for (k, slot) in a.iter_mut().chain(b.iter_mut()).enumerate() {
                        slot.write(self.inputs[c][k]);
                    }
                    // SAFETY: all n slots written.
                    unsafe { chunk.commit_all() };
                }
            }
        }
        if self.capturing {
            if let Some(tx) = self.record_tx.as_mut() {
                if tx.slots() >= frames * ch {
                    if let Ok(mut chunk) = tx.write_chunk_uninit(frames * ch) {
                        let (a, b) = chunk.as_mut_slices();
                        for (i, slot) in a.iter_mut().chain(b.iter_mut()).enumerate() {
                            slot.write(self.inputs[i % ch][i / ch]);
                        }
                        // SAFETY: all frames*ch slots written.
                        unsafe { chunk.commit_all() };
                    }
                    self.shared.record_frames.fetch_add(frames as u64, Ordering::Release);
                } else {
                    self.shared.record_dropped.fetch_add(frames as u64, Ordering::Relaxed);
                }
            }
        }
    }

    fn render_extras(&mut self, frames: usize) {
        // guide sequence (exact note onsets within the block)
        self.fx_l[..frames].fill(0.0);
        let mut done = false;
        if let Some(notes) = self.sequencer.notes.as_ref() {
            let start = self.sequencer.frame;
            let end = start + frames as u64;
            let mut cursor = 0usize;
            while self.sequencer.next < notes.len() && notes[self.sequencer.next].start < end {
                let n = notes[self.sequencer.next];
                let at = (n.start.saturating_sub(start) as usize).min(frames);
                self.tone.render_add(&mut self.fx_l[cursor..at]);
                self.tone.note_on(n.hz, n.amp, n.duration);
                cursor = at;
                self.sequencer.next += 1;
            }
            self.tone.render_add(&mut self.fx_l[cursor..frames]);
            if let Some(period) = self.sequencer.click_period {
                let mut cursor = 0usize;
                let mut tmp_end = frames;
                while self.sequencer.next_click < end as f64 {
                    let at = ((self.sequencer.next_click - start as f64).max(0.0) as usize).min(frames);
                    self.click.render_add(&mut self.fx_l[cursor..at]);
                    let beat = (self.sequencer.next_click / period).round() as u64;
                    self.click.trigger(beat.is_multiple_of(4), 0.25);
                    cursor = at;
                    self.sequencer.next_click += period;
                    tmp_end = frames;
                }
                self.click.render_add(&mut self.fx_l[cursor..tmp_end]);
            }
            self.sequencer.frame = end;
            let last_end = notes.last().map(|n| n.start + n.duration).unwrap_or(0);
            done = self.sequencer.next >= notes.len() && end > last_end + (self.sample_rate as u64 / 10);
        } else {
            self.tone.render_add(&mut self.fx_l[..frames]);
        }
        if done {
            if let Some(old) = self.sequencer.notes.take() {
                self.dispose(Garbage::Sequence(old));
            }
            self.sequencer.click_period = None;
        }
        // metronome on the transport
        if self.metronome.enabled && self.playing && self.sequencer.click_period.is_none() {
            let fpb = self.sample_rate as f64 * 60.0 / self.metronome.bpm.max(20.0) as f64;
            let start = self.position as f64;
            let next_beat = (start / fpb).ceil() * fpb;
            if next_beat < start + frames as f64 {
                let at = (next_beat - start) as usize;
                self.click.render_add(&mut self.fx_l[..at]);
                let beat = (next_beat / fpb).round() as u64;
                self.click.trigger(beat.is_multiple_of(self.metronome.beats_per_bar.max(1) as u64), self.metronome.gain);
                self.click.render_add(&mut self.fx_l[at..frames]);
            } else {
                self.click.render_add(&mut self.fx_l[..frames]);
            }
        } else if self.sequencer.click_period.is_none() {
            self.click.render_add(&mut self.fx_l[..frames]);
        }
        for k in 0..frames {
            self.out_l[k] += self.fx_l[k];
            self.out_r[k] += self.fx_l[k];
        }
        // direct input monitoring
        if let Some((c, g)) = self.direct_monitor {
            if let Some(inp) = self.inputs.get(c) {
                for (k, x) in inp[..frames].iter().enumerate() {
                    self.out_l[k] += x * g;
                    self.out_r[k] += x * g;
                }
            }
        }
        // preview playback (interleaved stereo ring)
        let mut finished = false;
        if let Some(p) = self.preview.as_mut() {
            // at most two segments: up to the loop end, then from the loop start
            let mut k0 = 0;
            for _ in 0..2 {
                if p.pos >= p.frames {
                    match p.loop_start {
                        Some(s) => p.pos = s,
                        None => break,
                    }
                }
                let remaining = p.frames.saturating_sub(p.pos) as usize;
                let want = (frames - k0).min(remaining);
                let avail = (p.consumer.slots() / 2).min(want);
                if let Ok(ch) = p.consumer.read_chunk(avail * 2) {
                    let (a, b) = ch.as_slices();
                    for k in 0..avail {
                        let get = |j: usize| if j < a.len() { a[j] } else { b[j - a.len()] };
                        self.out_l[k0 + k] += get(2 * k) * p.gain;
                        self.out_r[k0 + k] += get(2 * k + 1) * p.gain;
                    }
                    ch.commit_all();
                }
                p.pos += avail as u64;
                k0 += avail;
                if k0 >= frames || avail < want {
                    break;
                }
            }
            self.shared.preview_position.store(p.pos, Ordering::Relaxed);
            finished = p.loop_start.is_none() && p.pos >= p.frames;
        }
        if finished {
            if let Some(p) = self.preview.take() {
                self.dispose(Garbage::Stream(p.consumer));
            }
            self.shared.preview_playing.store(false, Ordering::Relaxed);
        }
    }

    /// Process one device callback. `output` is interleaved with
    /// `out_channels` channels; `input` supplies the matching input frames.
    pub fn process(&mut self, input: InputSource, output: &mut [f32], out_channels: usize) {
        let t0 = Instant::now();
        self.handle_commands();
        let out_channels = out_channels.max(1);
        let total = output.len() / out_channels;
        let mut done = 0;
        while done < total {
            let mut frames = (total - done).min(MAX_BLOCK);
            if self.playing {
                if let Some((_, le)) = self.loop_region {
                    if self.position < le {
                        frames = frames.min((le - self.position) as usize);
                    }
                }
            }
            self.read_input(frames, &input, done);
            self.tap_input(frames);

            let mut feed_underruns = 0;
            if self.playing {
                let mut feed = StreamFeed { slots: &mut self.streams, underruns: 0 };
                self.graph.process(frames, &mut feed, &self.inputs, &mut self.out_l, &mut self.out_r, Some(&self.shared));
                feed_underruns = feed.underruns;
            } else {
                self.graph.process(
                    frames,
                    &mut crate::graph::SilentFeed,
                    &self.inputs,
                    &mut self.out_l,
                    &mut self.out_r,
                    Some(&self.shared),
                );
            }
            if feed_underruns > 0 {
                self.shared.stream_underruns.fetch_add(feed_underruns, Ordering::Relaxed);
            }
            self.render_extras(frames);

            let out = &mut output[done * out_channels..(done + frames) * out_channels];
            for f in 0..frames {
                let (l, r) = (self.out_l[f], self.out_r[f]);
                let frame = &mut out[f * out_channels..(f + 1) * out_channels];
                if out_channels == 1 {
                    frame[0] = 0.5 * (l + r);
                } else {
                    frame[0] = l;
                    frame[1] = r;
                    for s in &mut frame[2..] {
                        *s = 0.0;
                    }
                }
            }

            if self.playing {
                self.position += frames as u64;
                if let Some((ls, le)) = self.loop_region {
                    if self.position >= le {
                        self.position = ls;
                    }
                }
            }
            done += frames;
        }
        self.shared.position.store(self.position, Ordering::Relaxed);
        self.shared.playing.store(self.playing, Ordering::Relaxed);
        self.shared.callbacks.fetch_add(1, Ordering::Relaxed);
        let budget = total as f32 / self.sample_rate;
        if budget > 0.0 {
            let load = t0.elapsed().as_secs_f32() / budget;
            let prev = self.shared.dsp_load();
            let smoothed = prev * 0.9 + load * 0.1;
            self.shared.dsp_load.store(smoothed.to_bits(), Ordering::Relaxed);
        }
    }
}
