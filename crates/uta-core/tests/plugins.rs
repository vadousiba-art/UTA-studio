//! Plugin Manager and plugin slots through `AppCore`, with the real helper
//! process and the UTA Test Gain fixture (param 0 = gain, 0.5 = unity,
//! value v → gain ×2v).

use std::sync::Arc;
use std::time::Duration;

use uta_audio::render::RenderOptions;
use uta_audio::wav::{read_wav, BitDepth};
use uta_audio::{BackendChoice, NullOptions};
use uta_core::jobs::JobState;
use uta_core::studio::EditOp;
use uta_core::{AppCore, AppPaths, Settings};
use uta_plugins::testing::{helper_binary, test_gain_bundle};
use uta_plugins::InstanceStatus;
use uta_project::{EffectKind, PluginRef};

const SR: u32 = 48_000;

fn app(dir: &std::path::Path) -> Arc<AppCore> {
    let paths = AppPaths::from_root(&dir.join("data"), &dir.join("projects"));
    let core = Arc::new(AppCore::with_backend(paths, Box::new(|_s: &Settings| BackendChoice::Null(NullOptions::new(SR)))).unwrap());
    core.start_engine().unwrap();
    core.set_plugin_helper(Some(helper_binary()));
    core
}

fn gain_value(core: &AppCore, track: Option<uuid::Uuid>, slot: uuid::Uuid) -> f64 {
    core.studio_plugin_params(track, slot).unwrap().values.iter().find(|v| v.id == 0).unwrap().value
}

#[test]
fn scan_insert_control_save_reopen_export_crash_and_disable() {
    let d = tempfile::tempdir().unwrap();
    let core = app(d.path());

    // scan a user folder containing the fixture bundle
    let bundle = test_gain_bundle();
    let mut s = core.settings();
    s.plugin_folders = vec![bundle.parent().unwrap().to_string_lossy().to_string()];
    core.update_settings(s).unwrap();
    let job = core.plugin_scan(false).unwrap();
    let info = core.jobs.wait(job, Duration::from_secs(60)).unwrap();
    let JobState::Done { result } = info.state else { panic!("{:?}", info.state) };
    assert!(result["effects"].as_u64().unwrap() >= 1, "{result}");
    let cat = core.plugin_catalog().unwrap();
    assert!(cat.available);
    let b = cat.bundles.iter().find(|b| b.path == bundle.to_string_lossy()).expect("fixture listed");
    assert!(b.enabled && b.error.is_none());
    let info = b.plugins[0].clone();
    assert_eq!(info.name, "UTA Test Gain");
    // a second scan skips the unchanged bundle
    let job = core.plugin_scan(false).unwrap();
    let JobState::Done { result } = core.jobs.wait(job, Duration::from_secs(60)).unwrap().state else { panic!() };
    assert!(result["unchanged"].as_u64().unwrap() >= 1);

    // project with an imported tone on the vocal track
    let st = core.studio_new("Plugins", None, Some(SR)).unwrap();
    let vocal = st.project.tracks[0].id;
    let src = d.path().join("tone.wav");
    let sig: Vec<f32> = (0..SR).map(|i| 0.4 * (i as f32 * 0.03).sin()).collect();
    uta_audio::wav::write_wav_f32(&src, SR, 1, &sig).unwrap();
    let job = core.studio_import(vec![src], Some(vocal), 0).unwrap();
    assert!(matches!(core.jobs.wait(job, Duration::from_secs(30)).unwrap().state, JobState::Done { .. }));

    let plugin = PluginRef {
        format: "vst3".into(),
        uid: info.uid.clone(),
        name: info.name.clone(),
        vendor: info.vendor.clone(),
        version: info.version.clone(),
        path: bundle.to_string_lossy().to_string(),
    };
    let st = core.studio_edit(EditOp::InsertPlugin { track: Some(vocal), plugin, index: None }, false).unwrap().unwrap();
    let slot = st.project.tracks[0].effects[0].id;
    assert_eq!(st.plugin_slots.len(), 1);
    assert_eq!(st.plugin_slots[0].status, InstanceStatus::Running, "{:?}", st.plugin_slots);
    assert_eq!(gain_value(&core, Some(vocal), slot), 0.5);

    // parameter edit (drag = transient edits + final), undo, redo
    for v in [0.45, 0.35, 0.3] {
        core.studio_edit(EditOp::SetPluginParam { track: Some(vocal), slot, id: 0, value: v }, true).unwrap();
    }
    let st = core.studio_edit(EditOp::SetPluginParam { track: Some(vocal), slot, id: 0, value: 0.25 }, false).unwrap().unwrap();
    assert_eq!(st.undo_label.as_deref(), Some("Plugin parameter"));
    assert_eq!(gain_value(&core, Some(vocal), slot), 0.25);
    core.studio_undo().unwrap();
    assert_eq!(gain_value(&core, Some(vocal), slot), 0.5, "undo restores the plugin");
    core.studio_redo().unwrap();
    assert_eq!(gain_value(&core, Some(vocal), slot), 0.25);

    // playback runs through the bridge
    core.studio_play(Some(0)).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    core.studio_stop().unwrap();

    // export: plugin applied (×0.5) and aligned (no bridge latency offline)
    let out = d.path().join("mix.wav");
    let job = core
        .studio_export(out.clone(), RenderOptions { start: 0, end: 0, tail_seconds: 0.0, sample_rate: None, depth: BitDepth::Float32, channels: 2, dither: false })
        .unwrap();
    let info = core.jobs.wait(job, Duration::from_secs(60)).unwrap();
    let JobState::Done { result } = info.state else { panic!("{:?}", info.state) };
    assert!(result["warnings"].as_array().unwrap().is_empty(), "{result}");
    let mix = read_wav(&out, None).unwrap();
    for k in [100usize, 5000, 20_000] {
        let l = mix.samples[k * 2];
        assert!((l - 0.5 * sig[k]).abs() < 1e-4, "frame {k}: {l} vs {}", 0.5 * sig[k]);
    }

    // save stores the plugin state; reopen restores it
    let saved = core.studio_save().unwrap();
    let EffectKind::Plugin { state, params, .. } = &saved.project.tracks[0].effects[0].effect else { panic!() };
    assert!(state.as_deref().unwrap().starts_with("vst3:"));
    assert_eq!(params.get(&0), Some(&0.25));
    let dir = std::path::PathBuf::from(&saved.path);
    core.studio_close(false).unwrap();
    let st = core.studio_open(&dir).unwrap();
    assert!(st.warnings.is_empty(), "{:?}", st.warnings);
    assert_eq!(st.plugin_slots[0].status, InstanceStatus::Running);
    assert_eq!(gain_value(&core, Some(vocal), slot), 0.25);

    // crash → dry passthrough + status, reload restores it
    core.debug_crash_plugin(slot);
    std::thread::sleep(Duration::from_millis(200));
    let st = core.studio_state().unwrap().unwrap();
    assert_eq!(st.plugin_slots[0].status, InstanceStatus::Crashed, "{:?}", st.plugin_slots);
    assert!(core.studio_plugin_params(Some(vocal), slot).is_err());
    core.studio_play(Some(0)).unwrap(); // the app keeps working
    core.studio_stop().unwrap();
    let st = core.studio_reload_plugin(Some(vocal), slot).unwrap();
    assert_eq!(st.plugin_slots[0].status, InstanceStatus::Running);
    assert_eq!(gain_value(&core, Some(vocal), slot), 0.25);

    // removing and undoing keeps the settings
    core.studio_edit(EditOp::RemoveEffect { track: Some(vocal), slot }, false).unwrap();
    assert!(core.studio_state().unwrap().unwrap().plugin_slots.is_empty());
    core.studio_undo().unwrap();
    assert_eq!(gain_value(&core, Some(vocal), slot), 0.25);

    // disabling the bundle unloads it; the slot keeps its data
    core.plugin_set_enabled(&bundle.to_string_lossy(), false).unwrap();
    let st = core.studio_state().unwrap().unwrap();
    assert_eq!(st.plugin_slots[0].status, InstanceStatus::Disabled);
    assert!(matches!(&st.project.tracks[0].effects[0].effect, EffectKind::Plugin { params, .. } if params.get(&0) == Some(&0.25)));
    core.plugin_set_enabled(&bundle.to_string_lossy(), true).unwrap();
    assert_eq!(core.studio_state().unwrap().unwrap().plugin_slots[0].status, InstanceStatus::Running);
    core.shutdown();
}
