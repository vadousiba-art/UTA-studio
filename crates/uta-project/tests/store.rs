use chrono::Utc;
use uta_project::edit::{self, TrackUpdate};
use uta_project::library::{AttemptEntry, PresetEntry, RecordingEntry};
use uta_project::*;

fn write_wav(path: &std::path::Path, frames: usize, sr: u32, channels: u16) {
    let spec = hound::WavSpec {
        channels,
        sample_rate: sr,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..frames * channels as usize {
        w.write_sample((i as f32 * 0.01).sin() * 0.1).unwrap();
    }
    w.finalize().unwrap();
}

fn sample_project(dir: &std::path::Path) -> Project {
    let mut p = Project::new("Demo 歌", 48_000);
    let t1 = edit::add_track(&mut p, Some("Lead Vocal".into()));
    let t2 = edit::add_track(&mut p, Some("Backing".into()));
    std::fs::create_dir_all(dir.join("audio")).unwrap();
    write_wav(&dir.join("audio/take1.wav"), 48_000, 48_000, 1);
    let asset = Asset {
        id: new_id(),
        file: "audio/take1.wav".into(),
        original_name: "take1".into(),
        kind: AssetKind::Recording,
        sample_rate: 48_000,
        channels: 1,
        frames: 48_000,
        created_at: Utc::now(),
    };
    let aid = asset.id;
    p.assets.push(asset);
    let c = edit::add_clip(&mut p, t1, aid, 12_000).unwrap();
    edit::trim_clip(&mut p, c, Some(14_000), Some(40_000)).unwrap();
    edit::update_track(&mut p, t1, &TrackUpdate { gain_db: Some(-3.5), pan: Some(-0.25), ..Default::default() }).unwrap();
    edit::update_track(&mut p, t2, &TrackUpdate { mute: Some(true), ..Default::default() }).unwrap();
    let eq = edit::insert_effect(&mut p, Some(t1), EffectSlot::builtin(uta_dsp::BuiltinKind::Eq), None).unwrap();
    edit::set_builtin_param(&mut p, Some(t1), eq, "mid2_gain", 4.5).unwrap();
    edit::insert_effect(&mut p, None, EffectSlot::builtin(uta_dsp::BuiltinKind::Reverb), None).unwrap();
    p
}

#[test]
fn save_and_reopen_round_trips_everything() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("Demo.utaproj");
    std::fs::create_dir_all(&dir).unwrap();
    let p = sample_project(&dir);
    // create() requires an empty folder; use a fresh folder and move audio in
    let dir2 = tmp.path().join("Demo2.utaproj");
    let mut store = ProjectStore::create(&dir2, &p).unwrap();
    std::fs::copy(dir.join("audio/take1.wav"), dir2.join("audio/take1.wav")).unwrap();
    store.save(&p, RevisionKind::Save).unwrap();
    drop(store);

    let (store, loaded) = ProjectStore::open(&dir2).unwrap();
    assert_eq!(loaded.project, p, "document identical after reopen");
    assert_eq!(loaded.skipped_corrupt, 0);
    assert!(loaded.newer_autosave.is_none());
    // track order, clip positions, gain, pan, effects
    let lp = &loaded.project;
    assert_eq!(lp.tracks[0].name, "Lead Vocal");
    assert_eq!(lp.tracks[1].name, "Backing");
    assert_eq!(lp.tracks[0].clips[0].position, 14_000);
    assert_eq!(lp.tracks[0].clips[0].source_offset, 2_000);
    assert_eq!(lp.tracks[0].gain_db, -3.5);
    assert_eq!(lp.tracks[0].pan, -0.25);
    assert!(lp.tracks[1].mute);
    match &lp.tracks[0].effects[0].effect {
        EffectKind::Builtin { params, .. } => assert_eq!(params["mid2_gain"], 4.5),
        _ => panic!(),
    }
    let checks = store.validate_assets(lp);
    assert_eq!(checks[0].status, AssetStatus::Ok);
}

#[test]
fn missing_and_unsafe_assets_are_reported_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("P.utaproj");
    let mut p = Project::new("P", 48_000);
    p.assets.push(Asset {
        id: new_id(),
        file: "audio/gone.wav".into(),
        original_name: "gone".into(),
        kind: AssetKind::Import,
        sample_rate: 48_000,
        channels: 2,
        frames: 100,
        created_at: Utc::now(),
    });
    p.assets.push(Asset {
        id: new_id(),
        file: "../../outside.wav".into(),
        original_name: "evil".into(),
        kind: AssetKind::Import,
        sample_rate: 48_000,
        channels: 2,
        frames: 100,
        created_at: Utc::now(),
    });
    let store = ProjectStore::create(&dir, &p).unwrap();
    // wrong sample rate file
    p.assets.push(Asset {
        id: new_id(),
        file: "audio/wrong.wav".into(),
        original_name: "wrong".into(),
        kind: AssetKind::Import,
        sample_rate: 48_000,
        channels: 1,
        frames: 100,
        created_at: Utc::now(),
    });
    write_wav(&dir.join("audio/wrong.wav"), 100, 44_100, 1);
    let checks = store.validate_assets(&p);
    assert_eq!(checks[0].status, AssetStatus::Missing);
    assert_eq!(checks[1].status, AssetStatus::UnsafePath);
    assert!(matches!(checks[2].status, AssetStatus::Mismatch { .. }));
}

#[test]
fn corrupted_latest_revision_falls_back_to_previous() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("C.utaproj");
    let mut p = Project::new("C", 48_000);
    let mut store = ProjectStore::create(&dir, &p).unwrap();
    edit::add_track(&mut p, Some("A".into()));
    store.save(&p, RevisionKind::Save).unwrap();
    edit::add_track(&mut p, Some("B".into()));
    store.save(&p, RevisionKind::Save).unwrap();
    store.debug_corrupt_latest().unwrap();
    drop(store);
    let (_, loaded) = ProjectStore::open(&dir).unwrap();
    assert_eq!(loaded.skipped_corrupt, 1);
    assert_eq!(loaded.project.tracks.len(), 1, "previous good revision loaded");
}

#[test]
fn autosave_newer_than_save_is_offered_for_recovery() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("A.utaproj");
    let mut p = Project::new("A", 48_000);
    let mut store = ProjectStore::create(&dir, &p).unwrap();
    edit::add_track(&mut p, Some("unsaved work".into()));
    let auto_id = store.save(&p, RevisionKind::Autosave).unwrap();
    drop(store); // "crash"
    let (store, loaded) = ProjectStore::open(&dir).unwrap();
    assert_eq!(loaded.project.tracks.len(), 0, "last manual save loads by default");
    let auto = loaded.newer_autosave.expect("autosave offered");
    assert_eq!(auto.id, auto_id);
    let (recovered, _) = store.load_revision(auto.id).unwrap();
    assert_eq!(recovered.tracks[0].name, "unsaved work");
}

#[test]
fn create_refuses_existing_project_and_open_refuses_non_project() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("X.utaproj");
    let p = Project::new("X", 48_000);
    ProjectStore::create(&dir, &p).unwrap();
    assert!(matches!(ProjectStore::create(&dir, &p), Err(StoreError::AlreadyExists(_))));
    assert!(matches!(ProjectStore::open(tmp.path()), Err(StoreError::NotAProject(_))));
}

#[test]
fn backup_copies_database_and_audio() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("B.utaproj");
    let mut p = Project::new("B", 48_000);
    let mut store = ProjectStore::create(&dir, &p).unwrap();
    write_wav(&dir.join("audio/x.wav"), 480, 48_000, 1);
    edit::add_track(&mut p, None);
    store.save(&p, RevisionKind::Save).unwrap();
    let dest = tmp.path().join("B-backup.utaproj");
    store.backup_to(&dest).unwrap();
    assert!(dest.join("audio/x.wav").is_file());
    let (_, loaded) = ProjectStore::open(&dest).unwrap();
    assert_eq!(loaded.project.tracks.len(), 1);
}

#[test]
fn library_round_trips() {
    let lib = Library::open_in_memory().unwrap();
    assert_eq!(lib.schema_version().unwrap(), 1);
    lib.set_setting("input_device", &"USB Mic").unwrap();
    assert_eq!(lib.get_setting::<String>("input_device").unwrap().as_deref(), Some("USB Mic"));
    assert_eq!(lib.get_setting::<String>("missing").unwrap(), None);

    lib.touch_project("p1", "Song A", "/tmp/a.utaproj", Utc::now(), Utc::now()).unwrap();
    lib.touch_project("p2", "Song B", "/tmp/b.utaproj", Utc::now(), Utc::now()).unwrap();
    assert_eq!(lib.recent_projects(10, None).unwrap().len(), 2);
    assert_eq!(lib.recent_projects(10, Some("B")).unwrap()[0].id, "p2");
    assert_eq!(lib.recent_projects(10, Some("%")).unwrap().len(), 0, "LIKE wildcards escaped");

    lib.add_recording(&RecordingEntry {
        id: "r1".into(),
        path: "/nonexistent/r1.wav".into(),
        created_at: Utc::now(),
        duration: 3.5,
        source: "analyzer".into(),
        title: "Take".into(),
        exercise_id: None,
        summary: serde_json::json!({"medianMidi": 57.0}),
    })
    .unwrap();
    assert_eq!(lib.recordings(10, Some("analyzer")).unwrap().len(), 1);
    assert_eq!(lib.recordings(10, Some("coach")).unwrap().len(), 0);

    for i in 0..3 {
        lib.add_attempt(&AttemptEntry {
            id: format!("a{i}"),
            created_at: Utc::now(),
            exercise_id: "pitch-match".into(),
            skill_id: "pitch".into(),
            level: 1,
            outcome: "success".into(),
            score: Some(0.9),
            confidence: Some(0.9),
            metrics: serde_json::json!({}),
            recording_id: Some("r1".into()),
            user_difficulty: Some(2),
            discomfort: false,
            notes: String::new(),
        })
        .unwrap();
    }
    assert_eq!(lib.attempts(Some("pitch"), None, 10).unwrap().len(), 3);
    let dir = tempfile::tempdir().unwrap();
    lib.delete_recording("r1", dir.path()).unwrap();
    assert!(lib.recording("r1").unwrap().is_none());
    assert!(lib.attempts(None, None, 10).unwrap().iter().all(|a| a.recording_id.is_none()));

    lib.save_preset(&PresetEntry {
        id: "x".into(),
        effect_kind: "eq".into(),
        name: "Bright vocal".into(),
        params: serde_json::json!({"high_gain": 3.0}),
        created_at: Utc::now(),
    })
    .unwrap();
    // same name overwrites
    lib.save_preset(&PresetEntry {
        id: "y".into(),
        effect_kind: "eq".into(),
        name: "Bright vocal".into(),
        params: serde_json::json!({"high_gain": 4.0}),
        created_at: Utc::now(),
    })
    .unwrap();
    let presets = lib.presets(Some("eq")).unwrap();
    assert_eq!(presets.len(), 1);
    assert_eq!(presets[0].params["high_gain"], 4.0);

    lib.delete_training_history().unwrap();
    assert!(lib.attempts(None, None, 10).unwrap().is_empty());
}
