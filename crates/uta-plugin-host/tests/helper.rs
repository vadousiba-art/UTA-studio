//! Drives the real `uta-plugin-host` binary against the UTA Test Gain VST3
//! fixture (built by scripts/build-test-plugin.sh, invoked here).

use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::OnceLock;

use uta_plugin_protocol::*;

fn fixture() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            let out = Command::new("bash").arg(root.join("scripts/build-test-plugin.sh")).output().expect("run build script");
            assert!(out.status.success(), "fixture build failed: {}", String::from_utf8_lossy(&out.stderr));
            PathBuf::from(String::from_utf8_lossy(&out.stdout).lines().last().unwrap().trim())
        })
        .clone()
}

struct Helper {
    child: Child,
    w: BufWriter<ChildStdin>,
    r: BufReader<ChildStdout>,
    scratch: Vec<u8>,
}

impl Helper {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_uta-plugin-host"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let w = BufWriter::new(child.stdin.take().unwrap());
        let r = BufReader::new(child.stdout.take().unwrap());
        Self { child, w, r, scratch: Vec::new() }
    }
    fn req(&mut self, req: &Request) -> Response {
        write_json(&mut self.w, req).unwrap();
        match read_frame(&mut self.r).unwrap() {
            Some(Frame::Json(j)) => parse_json(&j).unwrap(),
            other => panic!("unexpected {other:?}"),
        }
    }
    fn process(&mut self, params: &[(u32, f64)], l: &[f32], r: &[f32]) -> Option<(Vec<f32>, Vec<f32>)> {
        write_audio_request(&mut self.w, params, l, r, &mut self.scratch).ok()?;
        match read_frame(&mut self.r) {
            Ok(Some(Frame::AudioReply { left, right })) => Some((left, right)),
            _ => None,
        }
    }
}

fn load(h: &mut Helper, uid: &str, state: Option<String>) -> Response {
    h.req(&Request::Load {
        path: fixture().to_string_lossy().to_string(),
        uid: uid.into(),
        sample_rate: 48_000.0,
        max_block: 512,
        offline: false,
        component_state: state,
        controller_state: None,
    })
}

fn scan_uid(h: &mut Helper) -> PluginInfo {
    match h.req(&Request::Scan { path: fixture().to_string_lossy().to_string() }) {
        Response::Scanned { plugins } => {
            assert_eq!(plugins.len(), 1, "{plugins:?}");
            plugins.into_iter().next().unwrap()
        }
        r => panic!("{r:?}"),
    }
}

#[test]
fn scan_load_process_params_and_state() {
    let mut h = Helper::spawn();
    assert!(matches!(h.req(&Request::Hello), Response::Hello { version: PROTOCOL_VERSION, .. }));
    let info = scan_uid(&mut h);
    assert_eq!(info.name, "UTA Test Gain");
    assert_eq!(info.vendor, "UTA Vocal Lab Tests");
    assert!(!info.is_instrument);
    let r = load(&mut h, &info.uid, None);
    let Response::Loaded { params, latency_samples, .. } = r else { panic!("{r:?}") };
    assert_eq!(latency_samples, 0);
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].title, "Gain");
    assert_eq!(params[0].default_normalized, 0.5);

    // unity gain by default
    let input: Vec<f32> = (0..512).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
    let (l, r) = h.process(&[], &input, &input).unwrap();
    for (a, b) in l.iter().zip(&input) {
        assert!((a - b).abs() < 1e-6);
    }
    assert_eq!(l, r);
    // set gain to 0.25 (×0.5) via the audio request's parameter changes
    let (l, _) = h.process(&[(0, 0.25)], &input, &input).unwrap();
    assert!((l[100] - input[100] * 0.5).abs() < 1e-6);
    // the controller reflects it with the plugin's own text
    let Response::Params { values } = h.req(&Request::GetParams) else { panic!() };
    assert!((values[0].value - 0.25).abs() < 1e-9);
    assert_eq!(values[0].display, "-6.0 dB");
    // save state, reload in a fresh helper, gain is restored
    let Response::State { component_state, .. } = h.req(&Request::GetState) else { panic!() };
    assert!(matches!(h.req(&Request::Shutdown), Response::Ok));
    assert!(h.child.wait().unwrap().success());

    let mut h2 = Helper::spawn();
    let r = load(&mut h2, &info.uid, Some(component_state));
    assert!(matches!(r, Response::Loaded { .. }), "{r:?}");
    let (l, _) = h2.process(&[], &input, &input).unwrap();
    assert!((l[100] - input[100] * 0.5).abs() < 1e-6, "state restored");
    let Response::Params { values } = h2.req(&Request::GetParams) else { panic!() };
    assert!((values[0].value - 0.25).abs() < 1e-9, "controller synced from component state");
}

#[test]
fn invalid_files_are_rejected_with_messages() {
    let dir = tempfile::tempdir().unwrap();
    let bogus = dir.path().join("Fake.vst3");
    std::fs::write(&bogus, b"not a plugin").unwrap();
    let mut h = Helper::spawn();
    match h.req(&Request::Scan { path: bogus.to_string_lossy().to_string() }) {
        Response::Error { message } => assert!(message.contains("could not load"), "{message}"),
        r => panic!("{r:?}"),
    }
    let mut h = Helper::spawn();
    match h.req(&Request::Scan { path: dir.path().join("Missing.vst3").to_string_lossy().to_string() }) {
        Response::Error { message } => assert!(message.contains("does not exist")),
        r => panic!("{r:?}"),
    }
    let empty_bundle = dir.path().join("Empty.vst3");
    std::fs::create_dir_all(empty_bundle.join("Contents")).unwrap();
    let mut h = Helper::spawn();
    match h.req(&Request::Scan { path: empty_bundle.to_string_lossy().to_string() }) {
        Response::Error { message } => assert!(message.contains("no 64-bit"), "{message}"),
        r => panic!("{r:?}"),
    }
    // wrong uid
    let mut h = Helper::spawn();
    match load(&mut h, "00000000000000000000000000000000", None) {
        Response::Error { message } => assert!(message.contains("no longer contains"), "{message}"),
        r => panic!("{r:?}"),
    }
}

#[test]
fn crashing_plugin_only_kills_the_helper() {
    let mut h = Helper::spawn();
    let info = scan_uid(&mut h);
    assert!(matches!(load(&mut h, &info.uid, None), Response::Loaded { .. }));
    let input = vec![0.1f32; 256];
    assert!(h.process(&[], &input, &input).is_some());
    // "crash" parameter makes the plugin abort inside process()
    assert!(h.process(&[(1, 1.0)], &input, &input).is_none(), "no reply from a crashed plugin");
    let status = h.child.wait().unwrap();
    assert!(!status.success(), "helper died: {status:?}");
    // this test process (standing in for the app) is still alive and can
    // start a fresh helper
    let mut h2 = Helper::spawn();
    assert!(matches!(load(&mut h2, &info.uid, None), Response::Loaded { .. }));
}
