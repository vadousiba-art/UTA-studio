//! End-to-end tests of plugin hosting with the real helper process and the
//! UTA Test Gain fixture plugin (param 0 = gain, normalised 0.5 = unity,
//! value v → gain 2v).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use uta_audio::graph::{ChainProcessor, Graph, SilentFeed};
use uta_audio::graph_build::{build_graph, PluginFactory};
use uta_plugins::bridge::{Link, RtEndpoint, BRIDGE_LATENCY};
use uta_plugins::testing::{helper_binary, test_gain_bundle};
use uta_plugins::{scan_bundle, HelperClient, InstanceStatus, PluginManager};
use uta_project::{edit, EffectKind, EffectSlot, PluginRef, Project};

fn plugin_ref() -> PluginRef {
    let scan = scan_bundle(&helper_binary(), &test_gain_bundle());
    assert!(scan.error.is_none(), "{scan:?}");
    let info = &scan.plugins[0];
    PluginRef {
        format: "vst3".into(),
        uid: info.uid.clone(),
        name: info.name.clone(),
        vendor: info.vendor.clone(),
        version: info.version.clone(),
        path: test_gain_bundle().to_string_lossy().to_string(),
    }
}

fn plugin_slot(r: PluginRef, params: BTreeMap<u32, f64>) -> EffectSlot {
    EffectSlot { id: uta_project::new_id(), bypass: false, effect: EffectKind::Plugin { plugin: r, state: None, params } }
}

fn signal(start: usize, n: usize) -> Vec<f32> {
    (start..start + n).map(|i| ((i as f32) * 0.013).sin() * 0.5).collect()
}

/// Feed `blocks` blocks through `p` with real-time pacing; returns (input, output).
fn run_paced(p: &mut dyn ChainProcessor, blocks: usize, block: usize, pause: Duration) -> (Vec<f32>, Vec<f32>) {
    let mut input = Vec::new();
    let mut output = Vec::new();
    for b in 0..blocks {
        let x = signal(b * block, block);
        let mut l = x.clone();
        let mut r = x.clone();
        p.process(&mut l, &mut r);
        assert_eq!(l, r);
        input.extend_from_slice(&x);
        output.extend_from_slice(&l);
        std::thread::sleep(pause);
    }
    (input, output)
}

#[test]
fn scan_reports_the_fixture() {
    let r = plugin_ref();
    assert_eq!(r.name, "UTA Test Gain");
    assert_eq!(r.uid.len(), 32);
}

#[test]
fn realtime_bridge_is_sample_aligned_and_reports_latency() {
    let r = plugin_ref();
    let mut client = HelperClient::spawn(&helper_binary()).unwrap();
    let loaded = client
        .request(
            &uta_plugin_protocol::Request::Load {
                path: r.path.clone(),
                uid: r.uid.clone(),
                sample_rate: 48_000.0,
                max_block: 512,
                offline: false,
                component_state: None,
                controller_state: None,
            },
            Duration::from_secs(20),
        )
        .unwrap();
    assert!(matches!(loaded, uta_plugin_protocol::Response::Loaded { .. }));
    // gain ×0.5
    client
        .request(&uta_plugin_protocol::Request::SetParams { values: vec![(0, 0.25)] }, Duration::from_secs(5))
        .unwrap();
    let link = Link::new(client, 0);
    let mut ep = RtEndpoint::new(link.clone());
    assert_eq!(ep.latency_samples(), BRIDGE_LATENCY);
    let block = 256;
    let (input, output) = run_paced(&mut ep, 200, block, Duration::from_millis(2));
    let mut processed = 0;
    for k in BRIDGE_LATENCY..output.len() {
        let x = input[k - BRIDGE_LATENCY];
        let y = output[k];
        // every frame is either the processed (×0.5) or the dry fallback
        // signal, both exactly aligned to the reported latency
        let is_wet = (y - 0.5 * x).abs() < 1e-5;
        let is_dry = (y - x).abs() < 1e-6;
        assert!(is_wet || is_dry, "frame {k}: in {x} out {y}");
        if is_wet && x.abs() > 1e-3 {
            processed += 1;
        }
    }
    let total = output.len() - BRIDGE_LATENCY;
    assert!(processed as f64 > total as f64 * 0.9, "only {processed}/{total} frames processed");
    // the first BRIDGE_LATENCY frames are the priming silence
    assert!(output[..BRIDGE_LATENCY].iter().all(|v| *v == 0.0));
}

#[test]
fn manager_builds_graph_saves_state_and_survives_crash() {
    let r = plugin_ref();
    let mut project = Project::new("plugins", 48_000);
    let track = edit::add_track(&mut project, Some("Vocal".into()));
    let slot = plugin_slot(r.clone(), BTreeMap::new());
    let slot_id = slot.id;
    edit::insert_effect(&mut project, Some(track), slot, None).unwrap();

    let mut mgr = PluginManager::new(Some(helper_binary()));
    let g: Graph = build_graph(&project, 48_000.0, &mut mgr.factory());
    let st = mgr.statuses(&project);
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].status, InstanceStatus::Running, "{st:?}");
    assert_eq!(g.latency(), BRIDGE_LATENCY);

    // generic parameter control
    let descs = mgr.param_descs(slot_id).unwrap();
    assert_eq!(descs[0].title, "Gain");
    mgr.set_params(slot_id, &[(0, 0.25)]).unwrap();
    let v = mgr.param_values(slot_id).unwrap();
    assert!((v[0].value - 0.25).abs() < 1e-9);
    assert_eq!(v[0].display, "-6.0 dB");

    // state goes into the project
    assert!(mgr.store_states(&mut project));
    let EffectKind::Plugin { state, .. } = &project.tracks[0].effects[0].effect else { panic!() };
    assert!(state.as_deref().unwrap().starts_with("vst3:"));

    // a fresh manager (e.g. after reopening) restores the state
    drop(g);
    let mut mgr2 = PluginManager::new(Some(helper_binary()));
    let _g2 = build_graph(&project, 48_000.0, &mut mgr2.factory());
    let v = mgr2.param_values(slot_id).unwrap();
    assert!((v[0].value - 0.25).abs() < 1e-9, "state restored: {v:?}");

    // rebuilding reuses the running instance (same helper process)
    let pid_before = mgr2.statuses(&project)[0].message.clone();
    let _g3 = build_graph(&project, 48_000.0, &mut mgr2.factory());
    assert_eq!(mgr2.statuses(&project)[0].message, pid_before);

    // crash: slot reports it, graph passes audio (delayed dry)
    mgr2.crash_for_testing(slot_id);
    std::thread::sleep(Duration::from_millis(100));
    let mut probe = build_graph(&project, 48_000.0, &mut mgr2.factory());
    let st = mgr2.statuses(&project);
    assert_eq!(st[0].status, InstanceStatus::Crashed, "{st:?}");
    assert!(st[0].message.as_deref().unwrap().contains("Reload"));
    let (mut l, mut rr) = (vec![0.0; 256], vec![0.0; 256]);
    probe.process(256, &mut SilentFeed, &[], &mut l, &mut rr, None);
    assert!(mgr2.param_values(slot_id).is_err());

    // reload brings it back with its state
    mgr2.reset_slot(slot_id);
    let _g4 = build_graph(&project, 48_000.0, &mut mgr2.factory());
    assert_eq!(mgr2.statuses(&project)[0].status, InstanceStatus::Running);
    let v = mgr2.param_values(slot_id).unwrap();
    assert!((v[0].value - 0.25).abs() < 1e-9);

    // removing the slot shuts its helper down
    edit::remove_effect(&mut project, Some(track), slot_id).unwrap();
    mgr2.retain_project(&project);
    assert!(mgr2.param_values(slot_id).is_err());
}

#[test]
fn missing_disabled_and_unavailable_plugins_pass_through() {
    let mut r = plugin_ref();
    let mut project = Project::new("missing", 48_000);
    let track = edit::add_track(&mut project, None);
    let real_path = r.path.clone();
    r.path = "/nowhere/Gone.vst3".into();
    r.uid = "0123456789ABCDEF0123456789ABCDEF".into();
    edit::insert_effect(&mut project, Some(track), plugin_slot(r.clone(), BTreeMap::new()), None).unwrap();
    let mut mgr = PluginManager::new(Some(helper_binary()));
    let g = build_graph(&project, 48_000.0, &mut mgr.factory());
    assert_eq!(g.latency(), 0);
    let st = mgr.statuses(&project);
    assert_eq!(st[0].status, InstanceStatus::Missing);
    // the slot keeps its stored data untouched
    assert!(matches!(&project.tracks[0].effects[0].effect, EffectKind::Plugin { plugin, .. } if plugin.path == "/nowhere/Gone.vst3"));

    // a project from another computer is found through the scan catalog
    let good = plugin_ref();
    let mut p2 = Project::new("moved", 48_000);
    let t2 = edit::add_track(&mut p2, None);
    let mut moved = good.clone();
    moved.path = "C:/Other/Computer/UTA Test Gain.vst3".into();
    edit::insert_effect(&mut p2, Some(t2), plugin_slot(moved, BTreeMap::new()), None).unwrap();
    let mut known = HashMap::new();
    known.insert(good.uid.clone(), std::path::PathBuf::from(&real_path));
    mgr.set_catalog(known.clone(), HashSet::new());
    let _g = build_graph(&p2, 48_000.0, &mut mgr.factory());
    assert_eq!(mgr.statuses(&p2)[0].status, InstanceStatus::Running);

    // disabled
    let mut mgr_d = PluginManager::new(Some(helper_binary()));
    mgr_d.set_catalog(known, [real_path].into_iter().collect());
    let _g = build_graph(&p2, 48_000.0, &mut mgr_d.factory());
    assert_eq!(mgr_d.statuses(&p2)[0].status, InstanceStatus::Disabled);

    // no helper
    let mut none = PluginManager::new(None);
    let _g = build_graph(&p2, 48_000.0, &mut none.factory());
    assert_eq!(none.statuses(&p2)[0].status, InstanceStatus::Unavailable);
}

#[test]
fn offline_factory_processes_synchronously_without_bridge_latency() {
    let r = plugin_ref();
    let mut params = BTreeMap::new();
    params.insert(0u32, 0.25f64);
    let slot = plugin_slot(r, params);
    let mgr = PluginManager::new(Some(helper_binary()));
    let mut f = mgr.offline_factory();
    let mut p = f.create(None, &slot, 48_000.0).expect("loads");
    assert_eq!(p.latency_samples(), 0);
    let x = signal(0, 512);
    let (mut l, mut rr) = (x.clone(), x.clone());
    p.process(&mut l, &mut rr);
    for k in 0..512 {
        assert!((l[k] - 0.5 * x[k]).abs() < 1e-6);
    }
    assert!(f.errors().lock().is_empty());
}
