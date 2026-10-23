//! End-to-end engine tests on the null backend with synthetic signals.
//! They exercise the real audio-thread code paths (command ring, streaming,
//! recording ring, recorder thread, WAV writing) without hardware.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use uta_audio::core::SeqNote;
use uta_audio::graph_build::{build_graph, NoPlugins};
use uta_audio::recorder::RecordTarget;
use uta_audio::render::{render_to_wav, RenderOptions};
use uta_audio::source::build_sources;
use uta_audio::wav::{read_wav, wav_info, write_wav_f32, BitDepth};
use uta_audio::{AudioEngine, AudioError, BackendChoice, NullOptions};
use uta_project::edit::{self, TrackUpdate};
use uta_project::{new_id, Asset, AssetKind, EffectSlot, Project};

const SR: u32 = 48_000;

fn sine_gen(hz: f32, amp: f32) -> uta_audio::engine::InputGenerator {
    let mut phase = 0.0f32;
    Box::new(move |buf: &mut [f32], ch: usize| {
        for frame in buf.chunks_mut(ch) {
            let v = amp * (phase * std::f32::consts::TAU).sin();
            phase = (phase + hz / SR as f32) % 1.0;
            for s in frame.iter_mut() {
                *s = v;
            }
        }
    })
}

fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn add_asset(p: &mut Project, dir: &Path, name: &str, data: &[f32], channels: u16) -> uuid::Uuid {
    std::fs::create_dir_all(dir.join("audio")).unwrap();
    let file = format!("audio/{name}.wav");
    write_wav_f32(&dir.join(&file), SR, channels, data).unwrap();
    let a = Asset {
        id: new_id(),
        file,
        original_name: name.into(),
        kind: AssetKind::Import,
        sample_rate: SR,
        channels,
        frames: (data.len() / channels as usize) as u64,
        created_at: Utc::now(),
    };
    let id = a.id;
    p.assets.push(a);
    id
}

fn resolver(dir: PathBuf) -> impl Fn(&Asset) -> Option<PathBuf> {
    move |a: &Asset| Some(dir.join(&a.file)).filter(|p| p.is_file())
}

#[test]
fn capture_records_and_decodes_test_signal() {
    let d = tempfile::tempdir().unwrap();
    let mut opts = NullOptions::new(SR);
    opts.input_channels = 2;
    opts.input = Some(sine_gen(220.0, 0.5));
    let mut eng = AudioEngine::start(BackendChoice::Null(opts)).unwrap();
    assert!(eng.has_input());
    let path = d.path().join("take.wav");
    eng.capture(path.clone(), 0, None).unwrap();
    assert!(wait_until(Duration::from_secs(5), || eng.shared().record_frames.load(std::sync::atomic::Ordering::Relaxed) >= SR as u64 / 2));
    let out = eng.stop_recording().unwrap();
    assert!(out.result.errors.is_empty(), "{:?}", out.result.errors);
    assert_eq!(out.result.dropped_frames, 0);
    assert_eq!(out.result.missing_frames, 0);
    let f = &out.result.files[0];
    let info = wav_info(&f.path).unwrap();
    assert_eq!(info.sample_rate, SR);
    assert_eq!(info.channels, 1);
    assert_eq!(info.frames, f.frames);
    assert!(info.frames >= SR as u64 / 2, "{} frames", info.frames);
    // the decoded signal is the generated 220 Hz tone at amplitude 0.5
    let buf = read_wav(&f.path, None).unwrap();
    let a = uta_analysis::analyze_phrase(&buf.samples, SR, &Default::default());
    let med = a.pitch.median_midi.unwrap();
    assert!((med - 57.0).abs() < 0.05, "median midi {med}");
    let peak = buf.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!((peak - 0.5).abs() < 0.01);
    assert!(!eng.is_recording());
}

#[test]
fn input_clipping_is_flagged() {
    let mut opts = NullOptions::new(SR);
    opts.input_channels = 1;
    opts.input = Some(sine_gen(300.0, 1.2));
    let mut eng = AudioEngine::start(BackendChoice::Null(opts)).unwrap();
    assert!(wait_until(Duration::from_secs(3), || eng.status().inputs.first().map(|m| m.clipped).unwrap_or(false)));
    eng.reset_clip_indicators();
    // clipped again immediately because the input keeps clipping
    assert!(wait_until(Duration::from_secs(3), || eng.status().inputs[0].clipped));
}

#[test]
fn recording_without_input_is_refused() {
    let mut eng = AudioEngine::start(BackendChoice::Null(NullOptions::new(SR))).unwrap();
    let d = tempfile::tempdir().unwrap();
    let e = eng.capture(d.path().join("x.wav"), 0, None).unwrap_err();
    assert!(matches!(e, AudioError::NoDevice(_)), "{e}");
}

#[test]
fn playback_streams_clip_through_mixer() {
    let d = tempfile::tempdir().unwrap();
    let mut p = Project::new("play", SR);
    let t = edit::add_track(&mut p, None);
    // constant 0.5 for 1 s, starting at frame 4800
    let aid = add_asset(&mut p, d.path(), "dc", &vec![0.5; SR as usize], 1);
    edit::add_clip(&mut p, t, aid, 4800).unwrap();
    edit::update_track(&mut p, t, &TrackUpdate { gain_db: Some(-6.0206), pan: Some(1.0), ..Default::default() }).unwrap();

    let captured = Arc::new(Mutex::new(Vec::<f32>::new()));
    let cap = captured.clone();
    let mut opts = NullOptions::new(SR);
    opts.free_run = true;
    opts.output_sink = Some(Box::new(move |out: &[f32]| cap.lock().unwrap().extend_from_slice(out)));
    let mut eng = AudioEngine::start(BackendChoice::Null(opts)).unwrap();
    eng.set_graph(build_graph(&p, SR as f32, &mut NoPlugins)).unwrap();
    eng.set_sources(build_sources(&p, resolver(d.path().to_path_buf())));
    captured.lock().unwrap().clear();
    eng.play(0).unwrap();
    assert!(wait_until(Duration::from_secs(10), || eng.position() > SR as u64 + 9600));
    eng.stop().unwrap();
    let out = captured.lock().unwrap().clone();
    // find the first non-zero right-channel sample: should be the clip start
    let frames: Vec<(f32, f32)> = out.chunks(2).map(|c| (c[0], c[1])).collect();
    let first = frames.iter().position(|(_, r)| r.abs() > 1e-6).expect("audio played");
    // allow for blocks output before Play was processed
    let (l, r) = frames[first + 1000];
    assert!(l.abs() < 1e-6, "hard right: left silent ({l})");
    assert!((r - 0.25).abs() < 1e-3, "gain -6 dB: {r}");
    // the clip lasts exactly 1 s
    let last = frames.iter().rposition(|(_, r)| r.abs() > 1e-6).unwrap();
    assert_eq!(last - first + 1, SR as usize, "clip length preserved through streaming");
    assert_eq!(eng.status().stream_underruns, 0);
}

#[test]
fn loop_region_wraps_transport() {
    let d = tempfile::tempdir().unwrap();
    let mut p = Project::new("loop", SR);
    let t = edit::add_track(&mut p, None);
    let aid = add_asset(&mut p, d.path(), "a", &vec![0.1; SR as usize * 2], 1);
    edit::add_clip(&mut p, t, aid, 0).unwrap();
    let mut opts = NullOptions::new(SR);
    opts.free_run = true;
    let mut eng = AudioEngine::start(BackendChoice::Null(opts)).unwrap();
    eng.set_graph(build_graph(&p, SR as f32, &mut NoPlugins)).unwrap();
    eng.set_sources(build_sources(&p, resolver(d.path().to_path_buf())));
    eng.set_loop(Some((4800, 9600))).unwrap();
    eng.play(4800).unwrap();
    let mut max_seen = 0;
    let mut wrapped = false;
    let mut last = 0;
    wait_until(Duration::from_secs(5), || {
        let pos = eng.position();
        max_seen = max_seen.max(pos);
        if pos < last {
            wrapped = true;
        }
        last = pos;
        wrapped && eng.status().position > 0
    });
    eng.stop().unwrap();
    assert!(wrapped, "transport wrapped at loop end");
    assert!(max_seen < 9600, "never passed loop end: {max_seen}");
}

#[test]
fn studio_recording_places_take_with_known_start() {
    let d = tempfile::tempdir().unwrap();
    let mut p = Project::new("rec", SR);
    let t = edit::add_track(&mut p, None);
    edit::update_track(&mut p, t, &TrackUpdate { armed: Some(true), ..Default::default() }).unwrap();
    let mut opts = NullOptions::new(SR);
    opts.input_channels = 1;
    opts.input = Some(sine_gen(440.0, 0.3));
    opts.free_run = true;
    let mut eng = AudioEngine::start(BackendChoice::Null(opts)).unwrap();
    eng.set_graph(build_graph(&p, SR as f32, &mut NoPlugins)).unwrap();
    eng.set_sources(build_sources(&p, resolver(d.path().to_path_buf())));
    let path = d.path().join("rec.wav");
    eng.record(vec![RecordTarget { track_id: t.as_u128(), channel: 0, stereo: false, path: path.clone() }], 24_000).unwrap();
    assert!(wait_until(Duration::from_secs(10), || eng.position() > 24_000 + SR as u64));
    let out = eng.stop_recording().unwrap();
    assert_eq!(out.start_position, 24_000);
    assert_eq!(out.result.files.len(), 1);
    assert_eq!(out.result.files[0].track_id, t.as_u128());
    assert!(out.result.files[0].frames >= SR as u64);
    assert_eq!(out.result.missing_frames, 0);
    assert!(!eng.is_playing());
}

#[test]
fn device_loss_blocks_playback_but_recording_still_finalises() {
    let d = tempfile::tempdir().unwrap();
    let mut opts = NullOptions::new(SR);
    opts.input_channels = 1;
    opts.input = Some(sine_gen(200.0, 0.2));
    let mut eng = AudioEngine::start(BackendChoice::Null(opts)).unwrap();
    let path = d.path().join("lost.wav");
    eng.capture(path.clone(), 0, None).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    // simulate the device error callback
    eng.shared().device_lost.store(true, std::sync::atomic::Ordering::Release);
    let out = eng.stop_recording().unwrap();
    let info = wav_info(&path).unwrap();
    assert_eq!(info.frames, out.result.files[0].frames, "file is valid and complete");
    assert!(matches!(eng.play(0), Err(AudioError::Disconnected(_))));
    assert!(eng.status().device_lost);
}

#[test]
fn guide_sequence_and_capture_start_together() {
    let d = tempfile::tempdir().unwrap();
    let mut opts = NullOptions::new(SR);
    opts.input_channels = 1;
    opts.input = Some(sine_gen(196.0, 0.3));
    opts.free_run = true;
    let mut eng = AudioEngine::start(BackendChoice::Null(opts)).unwrap();
    let notes = vec![
        SeqNote { start: 0, duration: 4800, hz: 220.0, amp: 0.2 },
        SeqNote { start: 9600, duration: 4800, hz: 247.0, amp: 0.2 },
    ];
    eng.capture(d.path().join("g.wav"), 0, Some((notes, None))).unwrap();
    assert!(wait_until(Duration::from_secs(5), || eng.shared().record_frames.load(std::sync::atomic::Ordering::Relaxed) > 20_000));
    let out = eng.stop_recording().unwrap();
    assert!(out.result.files[0].frames > 20_000);
}

#[test]
fn preview_plays_file_to_end() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("p.wav");
    write_wav_f32(&path, SR, 1, &vec![0.3; 9600]).unwrap();
    let captured = Arc::new(Mutex::new(Vec::<f32>::new()));
    let cap = captured.clone();
    let mut opts = NullOptions::new(SR);
    opts.free_run = true;
    opts.output_sink = Some(Box::new(move |out: &[f32]| cap.lock().unwrap().extend_from_slice(out)));
    let mut eng = AudioEngine::start(BackendChoice::Null(opts)).unwrap();
    let frames = eng.preview(path, 0, 1.0).unwrap();
    assert_eq!(frames, 9600);
    assert!(wait_until(Duration::from_secs(5), || !eng.status().preview_playing && eng.status().preview_position >= 9600));
    let out = captured.lock().unwrap().clone();
    let n = out.chunks(2).filter(|c| (c[0] - 0.3).abs() < 1e-6).count();
    assert_eq!(n, 9600, "every preview frame played exactly once");
}

fn render_project(p: &Project, dir: &Path, opts: &RenderOptions) -> (uta_audio::render::RenderReport, Vec<f32>) {
    let graph = build_graph(p, SR as f32, &mut NoPlugins);
    let sources = Arc::new(build_sources(p, resolver(dir.to_path_buf())));
    let path = dir.join("mix.wav");
    let rep = render_to_wav(graph, sources, SR, opts, &path, |_| true).unwrap();
    let buf = read_wav(&path, None).unwrap();
    (rep, buf.samples)
}

fn opts(end: u64) -> RenderOptions {
    RenderOptions {
        start: 0,
        end,
        tail_seconds: 0.0,
        sample_rate: None,
        depth: BitDepth::Float32,
        channels: 2,
        dither: false,
    }
}

#[test]
fn export_renders_gain_pan_mute_solo() {
    let d = tempfile::tempdir().unwrap();
    let mut p = Project::new("mix", SR);
    let a = edit::add_track(&mut p, Some("A".into()));
    let b = edit::add_track(&mut p, Some("B".into()));
    let c = edit::add_track(&mut p, Some("C".into()));
    let aid = add_asset(&mut p, d.path(), "dc", &vec![0.2; 4800], 1);
    for t in [a, b, c] {
        edit::add_clip(&mut p, t, aid, 0).unwrap();
    }
    edit::update_track(&mut p, a, &TrackUpdate { pan: Some(-1.0), ..Default::default() }).unwrap();
    edit::update_track(&mut p, b, &TrackUpdate { pan: Some(1.0), gain_db: Some(-6.0206), ..Default::default() }).unwrap();
    edit::update_track(&mut p, c, &TrackUpdate { mute: Some(true), ..Default::default() }).unwrap();
    let (rep, s) = render_project(&p, d.path(), &opts(4800));
    assert!(rep.verified);
    assert_eq!(rep.frames, 4800);
    assert_eq!(rep.channels, 2);
    // L = A (0.2), R = B (0.1); C muted
    assert!((s[2000] - 0.2).abs() < 1e-4, "{}", s[2000]);
    assert!((s[2001] - 0.1).abs() < 1e-4, "{}", s[2001]);

    // solo B: only right channel
    edit::update_track(&mut p, b, &TrackUpdate { solo: Some(true), ..Default::default() }).unwrap();
    let (_, s) = render_project(&p, d.path(), &opts(4800));
    assert!(s[2000].abs() < 1e-6);
    assert!((s[2001] - 0.1).abs() < 1e-4);
}

#[test]
fn export_includes_effects_and_resamples_and_reports_clipping() {
    let d = tempfile::tempdir().unwrap();
    let mut p = Project::new("fx", SR);
    let t = edit::add_track(&mut p, None);
    let aid = add_asset(&mut p, d.path(), "dc", &vec![0.4; SR as usize], 1);
    edit::add_clip(&mut p, t, aid, 0).unwrap();
    let u = edit::insert_effect(&mut p, Some(t), EffectSlot::builtin(uta_dsp::BuiltinKind::Utility), None).unwrap();
    edit::set_builtin_param(&mut p, Some(t), u, "gain", 12.0).unwrap(); // 0.4 * ~3.98 = 1.59 → clips
    let mut o = opts(SR as u64);
    o.sample_rate = Some(44_100);
    o.depth = BitDepth::Int16;
    o.dither = true;
    let (rep, s) = render_project(&p, d.path(), &o);
    assert!(rep.verified);
    assert_eq!(rep.sample_rate, 44_100);
    assert!((rep.frames as i64 - 44_100).abs() <= 1, "{}", rep.frames);
    assert!(rep.clipped_samples > 0);
    assert!(!rep.warnings.is_empty());
    assert!(rep.peak_db > 3.0);
    // 16-bit output saturates at full scale
    assert!(s[20_000] > 0.99);

    // bypass the effect: level back to 0.4
    edit::set_effect_bypass(&mut p, Some(t), u, true).unwrap();
    let (rep, s) = render_project(&p, d.path(), &opts(SR as u64));
    assert_eq!(rep.clipped_samples, 0);
    assert!((s[20_000] - 0.4).abs() < 1e-4);
}

#[test]
fn export_cancellation_removes_partial_file() {
    let d = tempfile::tempdir().unwrap();
    let mut p = Project::new("c", SR);
    let t = edit::add_track(&mut p, None);
    let aid = add_asset(&mut p, d.path(), "a", &vec![0.1; SR as usize * 3], 1);
    edit::add_clip(&mut p, t, aid, 0).unwrap();
    let graph = build_graph(&p, SR as f32, &mut NoPlugins);
    let sources = Arc::new(build_sources(&p, resolver(d.path().to_path_buf())));
    let path = d.path().join("cancel.wav");
    let r = render_to_wav(graph, sources, SR, &opts(SR as u64 * 3), &path, |_| false);
    assert!(matches!(r, Err(AudioError::Cancelled)));
    assert!(!path.exists());
    assert!(!path.with_extension("wav.part").exists());
}
