//! Headless end-to-end workflows through `AppCore`, using the null audio
//! backend with a synthetic "singer" (a 220 Hz voice-like tone) as the
//! microphone.

use std::sync::Arc;
use std::time::{Duration, Instant};

use uta_audio::render::RenderOptions;
use uta_audio::wav::{read_wav, BitDepth};
use uta_audio::{BackendChoice, NullOptions};
use uta_core::jobs::JobState;
use uta_core::studio::EditOp;
use uta_core::{AppCore, AppPaths, Settings};
use uta_project::edit::TrackUpdate;

const SR: u32 = 48_000;

fn singer(hz: f32) -> uta_audio::engine::InputGenerator {
    let mut phase = 0.0f32;
    Box::new(move |buf: &mut [f32], ch: usize| {
        for frame in buf.chunks_mut(ch) {
            let p = phase * std::f32::consts::TAU;
            let v = 0.3 * (p.sin() + 0.5 * (2.0 * p).sin() + 0.25 * (3.0 * p).sin());
            phase = (phase + hz / SR as f32) % 1.0;
            frame.iter_mut().for_each(|s| *s = v);
        }
    })
}

fn app(dir: &std::path::Path, free_run: bool) -> Arc<AppCore> {
    let paths = AppPaths::from_root(&dir.join("data"), &dir.join("projects"));
    let core = AppCore::with_backend(
        paths,
        Box::new(move |s: &Settings| {
            let mut o = NullOptions::new(SR);
            if s.microphone_enabled {
                o.input_channels = 1;
                o.input = Some(singer(220.0));
            }
            o.free_run = free_run;
            BackendChoice::Null(o)
        }),
    )
    .unwrap();
    let core = Arc::new(core);
    core.start_engine().unwrap();
    let mut s = core.settings();
    s.microphone_enabled = true;
    core.update_settings(s).unwrap();
    core
}

fn wait(d: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + d;
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn microphone_is_off_until_enabled() {
    let d = tempfile::tempdir().unwrap();
    let paths = AppPaths::from_root(&d.path().join("data"), &d.path().join("projects"));
    let core = AppCore::with_backend(paths, Box::new(|_s: &Settings| BackendChoice::Null(NullOptions::new(SR)))).unwrap();
    core.start_engine().unwrap();
    assert!(!core.settings().microphone_enabled, "no microphone access by default");
    let e = core.analyzer_start().unwrap_err();
    assert!(e.to_string().contains("Microphone access is off"));
}

#[test]
fn analyzer_records_analyzes_and_compares() {
    let d = tempfile::tempdir().unwrap();
    let core = app(d.path(), false);
    core.analyzer_start().unwrap();
    assert!(core.capture_status().active);
    std::thread::sleep(Duration::from_millis(1500));
    let r1 = core.analyzer_stop().unwrap();
    assert!(!core.capture_status().active);
    let med = r1.analysis.pitch.median_midi.unwrap();
    assert!((med - 57.0).abs() < 0.1, "the synthetic singer is A3 (57): {med}");
    assert!(r1.analysis.duration > 1.2);
    assert!(std::path::Path::new(&r1.recording.path).is_file());
    assert!(!r1.feedback.went_well.is_empty());

    core.analyzer_start().unwrap();
    std::thread::sleep(Duration::from_millis(800));
    let r2 = core.analyzer_stop().unwrap();
    let list = core.recordings(Some("analyzer"), 10).unwrap();
    assert_eq!(list.len(), 2);
    let cmp = core.compare_recordings(&r1.recording.id, &r2.recording.id).unwrap();
    assert!(cmp.metrics.iter().any(|m| m.key == "wobble"));
    let c = cmp.contour.expect("contours compared");
    assert!(c.mean_abs_pitch_diff_cents.unwrap() < 10.0, "same pitch in both takes");
    // cached analysis reloads
    let again = core.recording_analysis(&r1.recording.id).unwrap();
    assert_eq!(again.analysis.notes.len(), r1.analysis.notes.len());
    // waveform
    let wf = core.waveform(std::path::Path::new(&r1.recording.path), 0, None, 100).unwrap();
    assert_eq!(wf.len(), 200);
    assert!(wf.iter().any(|v| v.abs() > 0.2));
    // deletion removes the file
    core.delete_recording(&r2.recording.id).unwrap();
    assert!(!std::path::Path::new(&r2.recording.path).exists());
    assert_eq!(core.recordings(Some("analyzer"), 10).unwrap().len(), 1);
}

#[test]
fn coach_attempt_is_graded_saved_and_scheduled() {
    let d = tempfile::tempdir().unwrap();
    let core = app(d.path(), false);
    let ov = core.coach_overview().unwrap();
    let rec = ov.recommendation.expect("a first exercise is recommended");
    assert!(ov.disclaimer.contains("not a substitute"));
    // Use a sustain at a range centred on the singer's 220 Hz (A3 = 57)
    let mut s = core.settings();
    s.voice_range = uta_coach::exercises::VoiceRange { low: 52.0, high: 62.0 };
    core.update_settings(s).unwrap();
    let plan = core.coach_plan("pitch_match", Some(1)).unwrap();
    assert!(!plan.targets.is_empty());
    core.coach_start_attempt(plan.clone()).unwrap();
    std::thread::sleep(Duration::from_secs_f32(plan.record_seconds + 0.2));
    let res = core.coach_stop_attempt(Some(2), None).unwrap();
    // the synthetic singer holds A3 throughout; it is graded against the
    // target note — conclusive either way
    assert_ne!(res.evaluation.outcome, uta_coach::evaluate::Outcome::Inconclusive, "{:#?}", res.evaluation.feedback);
    assert!(res.next_review.is_some());
    assert!(std::path::Path::new(&res.recording.path).is_file());
    let ov2 = core.coach_overview().unwrap();
    assert_eq!(ov2.recent.len(), 1);
    assert!(ov2.skills.iter().any(|s| s.id == "pitch_matching" && s.level >= 1));
    assert!(!ov2.reviews.is_empty());
    let _ = rec;

    // discomfort during an attempt stops it and pauses demanding work
    let plan = core.coach_plan("sustain_hold", Some(1));
    let plan = match plan {
        Ok(p) => p,
        Err(_) => core.coach_plan("pitch_match", Some(1)).unwrap(),
    };
    core.coach_start_attempt(plan).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let health = core.coach_report_discomfort(Some("throat felt scratchy".into())).unwrap();
    assert!(!core.capture_status().active, "recording stopped");
    assert!(health.resting_until.is_some());
    assert!(health.message.is_some());
    let ov3 = core.coach_overview().unwrap();
    assert_eq!(ov3.recent[0].outcome, "stopped");
    assert!(ov3.recent[0].discomfort);
    if let Some(r) = ov3.recommendation {
        assert!(!uta_coach::progression::is_demanding(&r.skill));
    }
}

#[test]
fn range_check_updates_voice_range() {
    let d = tempfile::tempdir().unwrap();
    let core = app(d.path(), false);
    core.range_start(true).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let r = core.range_stop().unwrap();
    assert_eq!(r.midi, Some(57.0));
    assert_eq!(core.settings().voice_range.high, 57.0);
    assert!(core.settings().voice_range_measured);
}

#[test]
fn studio_record_edit_undo_save_reopen_export() {
    let d = tempfile::tempdir().unwrap();
    let core = app(d.path(), false);
    let st = core.studio_new("Demo 歌", None, None).unwrap();
    let vocal = st.project.tracks[0].id;
    assert!(st.project.tracks[0].armed);

    // record ~1 s
    core.studio_record(Some(0)).unwrap();
    std::thread::sleep(Duration::from_millis(1200));
    let res = core.studio_stop_recording().unwrap();
    assert_eq!(res.clips.len(), 1);
    let st = core.studio_state().unwrap().unwrap();
    let clip = st.project.tracks[0].clips[0].clone();
    assert!(clip.length > SR as u64 / 2, "{}", clip.length);
    assert!(st.can_undo);
    assert!(st.dirty);

    // second take over the first → take group
    core.studio_record(Some(0)).unwrap();
    std::thread::sleep(Duration::from_millis(900));
    core.studio_stop_recording().unwrap();
    let st = core.studio_state().unwrap().unwrap();
    assert_eq!(st.project.tracks[0].take_groups.len(), 1, "overlapping takes are grouped");
    let g = st.project.tracks[0].take_groups[0].clone();
    // comp: first take for the first 0.25 s
    core.studio_edit(EditOp::SetComp { track: vocal, group: g.id, start: 0, end: 12_000, take: g.takes[0] }, false).unwrap();
    core.studio_edit(EditOp::FlattenComp { track: vocal, group: g.id }, false).unwrap();
    let st = core.studio_state().unwrap().unwrap();
    assert!(st.project.tracks[0].take_groups.is_empty());

    // move and trim the first clip, then undo / redo
    let c0 = st.project.tracks[0].clips[0].id;
    core.studio_edit(EditOp::MoveClip { clip: c0, position: 4800, track: None }, false).unwrap();
    core.studio_edit(EditOp::TrimClip { clip: c0, start: None, end: Some(9600) }, false).unwrap();
    let moved = core.studio_state().unwrap().unwrap().project;
    let mc = moved.tracks[0].clips.iter().find(|c| c.id == c0).unwrap().clone();
    assert_eq!((mc.position, mc.end()), (4800, 9600));
    core.studio_undo().unwrap();
    let u = core.studio_state().unwrap().unwrap().project;
    assert_eq!(u.tracks[0].clips.iter().find(|c| c.id == c0).unwrap().position, 4800);
    assert_ne!(u.tracks[0].clips.iter().find(|c| c.id == c0).unwrap().end(), 9600);
    core.studio_redo().unwrap();
    assert_eq!(core.studio_state().unwrap().unwrap().project, moved);

    // fader drag: several transient edits = one undo step
    for g in [-1.0, -2.0, -3.0, -4.5] {
        core.studio_edit(EditOp::UpdateTrack { track: vocal, update: TrackUpdate { gain_db: Some(g), ..Default::default() } }, true).unwrap();
    }
    core.studio_edit(EditOp::UpdateTrack { track: vocal, update: TrackUpdate { gain_db: Some(-5.0), pan: Some(0.3), ..Default::default() } }, false).unwrap();
    let st = core.studio_state().unwrap().unwrap();
    assert_eq!(st.project.tracks[0].gain_db, -5.0);
    assert_eq!(st.undo_label.as_deref(), Some("Track change"));
    core.studio_undo().unwrap();
    assert_eq!(core.studio_state().unwrap().unwrap().project.tracks[0].gain_db, 0.0, "whole drag undone at once");
    core.studio_redo().unwrap();

    // effects
    let st = core.studio_edit(EditOp::InsertEffect { track: Some(vocal), effect: "eq".into(), index: None }, false).unwrap().unwrap();
    let eq = st.project.tracks[0].effects[0].id;
    core.studio_edit(EditOp::SetEffectParam { track: Some(vocal), slot: eq, param: "high_gain".into(), value: 3.0 }, false).unwrap();
    core.studio_edit(EditOp::InsertEffect { track: None, effect: "reverb".into(), index: None }, false).unwrap();
    core.studio_edit(EditOp::AddTrack { name: Some("Harmony".into()) }, false).unwrap();

    // playback runs
    core.studio_play(Some(0)).unwrap();
    assert!(wait(Duration::from_secs(3), || core.engine_status().unwrap().position > 4800));
    core.studio_stop().unwrap();

    // save, close, reopen
    let saved = core.studio_save().unwrap();
    assert!(!saved.dirty);
    let path = std::path::PathBuf::from(&saved.path);
    let before = saved.project.clone();
    core.studio_close(false).unwrap();
    assert!(core.studio_state().unwrap().is_none());
    let reopened = core.studio_open(&path).unwrap();
    assert_eq!(reopened.project, before, "identical after reopen");
    assert!(reopened.asset_checks.is_empty(), "all audio present: {:?}", reopened.asset_checks);
    assert_eq!(core.recent_projects(None).unwrap()[0].name, "Demo 歌");

    // export and decode
    let out = d.path().join("mix.wav");
    let job = core
        .studio_export(out.clone(), RenderOptions { start: 0, end: 0, tail_seconds: 0.5, sample_rate: None, depth: BitDepth::Int24, channels: 2, dither: false })
        .unwrap();
    let info = core.jobs.wait(job, Duration::from_secs(30)).unwrap();
    let JobState::Done { result } = info.state else { panic!("export failed: {:?}", info.state) };
    assert_eq!(result["verified"], true);
    let decoded = read_wav(&out, None).unwrap();
    assert_eq!(decoded.channels, 2);
    assert_eq!(decoded.sample_rate, SR);
    let expected = before.length_frames() + SR as u64 / 2;
    assert_eq!(decoded.frames() as u64, expected);
    let peak = decoded.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.05, "the export contains the recorded voice ({peak})");
}

#[test]
fn autosave_recovery_after_crash() {
    let d = tempfile::tempdir().unwrap();
    let core = app(d.path(), true);
    let st = core.studio_new("Crashy", None, None).unwrap();
    let path = std::path::PathBuf::from(&st.path);
    core.studio_edit(EditOp::AddTrack { name: Some("Unsaved".into()) }, false).unwrap();
    core.studio_autosave_now().unwrap();
    drop(core); // simulated crash: no save, no close
    let core2 = app(d.path(), true);
    let st = core2.studio_open(&path).unwrap();
    assert_eq!(st.project.tracks.len(), 1, "last manual save opens");
    let auto = st.newer_autosave.expect("autosave offered");
    let rec = core2.studio_load_revision(auto.id).unwrap();
    assert_eq!(rec.project.tracks.len(), 2);
    assert!(rec.can_undo, "recovery can be undone");
}

#[test]
fn import_and_missing_asset_relink() {
    let d = tempfile::tempdir().unwrap();
    let core = app(d.path(), true);
    core.studio_new("Imp", None, Some(48_000)).unwrap();
    let src = d.path().join("backing.wav");
    let sig: Vec<f32> = (0..44_100 * 2).map(|i| 0.2 * (i as f32 * 0.01).sin()).collect();
    uta_audio::wav::write_wav_f32(&src, 44_100, 2, &sig).unwrap();
    let job = core.studio_import(vec![src.clone()], None, 0).unwrap();
    let info = core.jobs.wait(job, Duration::from_secs(30)).unwrap();
    assert!(matches!(info.state, JobState::Done { .. }), "{:?}", info.state);
    let st = core.studio_state().unwrap().unwrap();
    assert_eq!(st.project.tracks.len(), 2, "import creates a track");
    let asset = st.project.assets[0].clone();
    assert_eq!(asset.sample_rate, 48_000, "converted to project rate");
    // 88 200 interleaved stereo samples = 1 s at 44.1 kHz → 48 000 frames
    assert!((asset.frames as i64 - 48_000).abs() <= 2, "{}", asset.frames);
    // delete the file behind the app's back
    let path = std::path::PathBuf::from(&st.path).join(&asset.file);
    core.studio_save().unwrap();
    std::fs::remove_file(&path).unwrap();
    let dir = std::path::PathBuf::from(&st.path);
    core.studio_close(false).unwrap();
    let st = core.studio_open(&dir).unwrap();
    assert_eq!(st.asset_checks.len(), 1);
    assert!(st.warnings.iter().any(|w| w.contains("missing")));
    // the project still plays (missing clip is silent) and relinks
    core.studio_play(Some(0)).unwrap();
    core.studio_stop().unwrap();
    let st = core.studio_relink_asset(asset.id, &src).unwrap();
    assert!(st.asset_checks.is_empty());
}

#[test]
fn engine_restart_on_settings_change_keeps_project_loaded() {
    let d = tempfile::tempdir().unwrap();
    let core = app(d.path(), true);
    core.studio_new("R", None, None).unwrap();
    let mut s = core.settings();
    s.buffer_frames = Some(512);
    core.update_settings(s).unwrap();
    core.studio_play(Some(0)).unwrap();
    core.studio_stop().unwrap();
    assert!(core.info().engine.is_some());
}
