//! Song Lab workflow through `AppCore` with the null backend and a
//! synthetic singer (220 Hz) as the microphone.

use std::sync::Arc;
use std::time::Duration;

use uta_audio::{BackendChoice, NullOptions};
use uta_core::jobs::JobState;
use uta_core::songlab::{SongPhrase, SongTrack, SongUpdate};
use uta_core::{AppCore, AppPaths, Settings};

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

fn app(dir: &std::path::Path) -> Arc<AppCore> {
    app_with(dir, 220.0)
}

/// `hz` 0 = a silent microphone.
fn app_with(dir: &std::path::Path, hz: f32) -> Arc<AppCore> {
    let paths = AppPaths::from_root(&dir.join("data"), &dir.join("projects"));
    let core = Arc::new(
        AppCore::with_backend(
            paths,
            Box::new(move |s: &Settings| {
                let mut o = NullOptions::new(SR);
                if s.microphone_enabled {
                    o.input_channels = 1;
                    o.input = Some(if hz > 0.0 { singer(hz) } else { Box::new(|b: &mut [f32], _ch: usize| b.fill(0.0)) });
                }
                BackendChoice::Null(o)
            }),
        )
        .unwrap(),
    );
    core.start_engine().unwrap();
    let mut s = core.settings();
    s.microphone_enabled = true;
    core.update_settings(s).unwrap();
    core
}

fn done(core: &AppCore, job: u64) -> serde_json::Value {
    let info = core.jobs.wait(job, Duration::from_secs(60)).unwrap();
    match info.state {
        JobState::Done { result } => result,
        s => panic!("job failed: {s:?}"),
    }
}

fn tone(path: &std::path::Path, rate: u32, seconds: f32, hz: f32, channels: u16) {
    let n = (seconds * rate as f32) as usize;
    let mut v = Vec::with_capacity(n * channels as usize);
    for i in 0..n {
        let p = i as f32 * hz / rate as f32 * std::f32::consts::TAU;
        let x = 0.2 * (p.sin() + 0.4 * (2.0 * p).sin());
        for _ in 0..channels {
            v.push(x);
        }
    }
    uta_audio::wav::write_wav_f32(path, rate, channels, &v).unwrap();
}

#[test]
fn song_lab_import_lyrics_slow_loop_attempt_compare_delete() {
    let d = tempfile::tempdir().unwrap();
    let core = app(d.path());

    let song = core.song_create("練習曲", "Me").unwrap();
    let id = song.id.clone();
    assert!(song.data.backing.is_none());

    // backing (44.1 kHz stereo, converted) and reference vocal (220 Hz)
    let backing = d.path().join("backing.wav");
    tone(&backing, 44_100, 4.0, 110.0, 2);
    done(&core, core.song_import_audio(&id, SongTrack::Backing, backing).unwrap());
    let reference = d.path().join("ref.wav");
    tone(&reference, SR, 4.0, 220.0, 1);
    done(&core, core.song_import_audio(&id, SongTrack::Reference, reference).unwrap());
    let v = core.song_get(&id).unwrap();
    let b = v.data.backing.as_ref().unwrap();
    assert_eq!(b.sample_rate, SR);
    assert!((b.seconds() - 4.0).abs() < 0.01);
    assert!(v.ready_speeds.contains(&1.0));
    assert!(!v.ready_speeds.contains(&0.75));

    // lyrics with romaji and readings
    let v = core.song_import_lyrics(&id, "[00:00.50]さくら さくら\n[00:01.50]夜空\n[00:03.00]").unwrap();
    assert_eq!(v.data.lines.len(), 2);
    assert_eq!(v.romaji[0].romaji, "sakura sakura");
    assert_eq!(v.romaji[0].morae, Some(6));
    assert!(v.romaji[1].has_kanji);
    let mut lines = v.data.lines.clone();
    lines[1].reading = "よぞら".into();
    let phrase = SongPhrase { id: "p1".into(), name: "Verse 1".into(), start: 0.5, end: 1.5, line_ids: vec![lines[0].id.clone()], notes: String::new() };
    let v = core
        .song_update(&id, SongUpdate { lines: Some(lines), phrases: Some(vec![phrase]), notes: Some("breathe before 夜空".into()), ..Default::default() })
        .unwrap();
    assert_eq!(v.romaji[1].romaji, "yozora");
    assert_eq!(v.data.notes, "breathe before 夜空");

    // invalid phrase is rejected
    let bad = SongPhrase { id: "x".into(), name: "bad".into(), start: 2.0, end: 1.0, line_ids: vec![], notes: String::new() };
    assert!(core.song_update(&id, SongUpdate { phrases: Some(vec![bad]), ..Default::default() }).is_err());

    // slow-down render (pitch unchanged), then a looped phrase plays
    assert!(core.song_play(&id, SongTrack::Backing, 0.5, Some(1.5), true, 0.75).is_err(), "not prepared yet");
    let r = done(&core, core.song_prepare(&id, 0.75).unwrap());
    assert_eq!(r["speed"], 0.75);
    let v = core.song_get(&id).unwrap();
    assert!(v.ready_speeds.contains(&0.75), "{:?}", v.ready_speeds);
    let pb = core.song_play(&id, SongTrack::Backing, 0.5, Some(1.5), true, 0.75).unwrap();
    assert_eq!(pb.start_frame, 32_000);
    assert_eq!(pb.end_frame, 96_000);
    std::thread::sleep(Duration::from_millis(300));
    assert!(core.engine_status().unwrap().preview_playing);
    core.song_stop().unwrap();
    // the stretched render keeps the pitch: analyse it
    let slow = std::fs::read_dir(d.path().join("data").join("songs").join(&id).join("cache"))
        .unwrap()
        .flatten()
        .find(|f| f.file_name().to_string_lossy().starts_with("reference-48000-75"))
        .unwrap()
        .path();
    let buf = uta_audio::wav::read_wav(&slow, None).unwrap();
    assert!((buf.frames() as f64 / SR as f64 - 4.0 / 0.75).abs() < 0.05, "{}", buf.frames());
    let a = uta_analysis::analyze_phrase(&buf.to_mono(), SR, &Default::default());
    let median = a.pitch.median_midi.unwrap();
    assert!((median - 57.0).abs() < 0.2, "pitch preserved: {median}");

    // attempt at 75 % with backing; the singer sings 220 Hz like the reference
    let pb = core.song_attempt_start(&id, "p1", 0.75, true).unwrap();
    assert_eq!(pb.start_frame, 0, "preroll is clamped at the song start");
    assert!(core.capture_status().kind.unwrap().starts_with("song:"));
    std::thread::sleep(Duration::from_millis(2400));
    let res = core.song_attempt_stop().unwrap();
    assert!(!core.engine_status().unwrap().preview_playing, "backing stops with the attempt");
    let c = res.comparison.expect("compared with the reference");
    assert!(c.mean_abs_pitch_diff_cents.unwrap() < 30.0, "{c:?}");
    assert!(res.contour.iter().any(|f| f.time > 1.0 && f.time < 1.5), "contour mapped to song time");
    assert!(res.notes.iter().any(|n| n.contains("75%")));
    assert!(res.recording.title.contains("Verse 1"));
    let v = core.song_get(&id).unwrap();
    assert_eq!(v.attempts.len(), 1);
    let rc = core.song_reference_contour(&id, "p1").unwrap();
    assert!(rc.first().unwrap().time >= 0.5 && rc.last().unwrap().time <= 1.5 + 0.02);

    // songs list and deletion (files and attempts)
    assert_eq!(core.songs().unwrap()[0].phrases, 1);
    let rec_path = std::path::PathBuf::from(&res.recording.path);
    assert!(rec_path.is_file());
    core.song_delete(&id).unwrap();
    assert!(core.songs().unwrap().is_empty());
    assert!(!rec_path.exists());
    assert!(!d.path().join("data").join("songs").join(&id).exists());
    assert!(core.song_get(&id).is_err());
    assert!(core.song_get("../../etc").is_err());
}

#[test]
fn silent_attempt_is_not_compared() {
    let d = tempfile::tempdir().unwrap();
    let core = app_with(d.path(), 0.0);
    let id = core.song_create("Quiet", "").unwrap().id;
    let reference = d.path().join("ref.wav");
    tone(&reference, SR, 3.0, 220.0, 1);
    done(&core, core.song_import_audio(&id, SongTrack::Reference, reference).unwrap());
    let phrase = SongPhrase { id: "p".into(), name: "P".into(), start: 0.2, end: 1.2, line_ids: vec![], notes: String::new() };
    core.song_update(&id, SongUpdate { phrases: Some(vec![phrase]), ..Default::default() }).unwrap();
    core.song_attempt_start(&id, "p", 1.0, false).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let res = core.song_attempt_stop().unwrap();
    assert!(res.comparison.is_none(), "silence must not be scored against the reference");
    assert!(res.notes.iter().any(|n| n.contains("Not enough singing")), "{:?}", res.notes);
}
