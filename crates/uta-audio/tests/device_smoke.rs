//! Smoke test against the real device backend (cpal). Opt-in because CI
//! machines usually have no audio hardware:
//!
//!     UTA_DEVICE_TESTS=1 cargo test -p uta-audio --test device_smoke
//!
//! On Linux without a sound card, an ALSA `null` default device
//! (`pcm.!default { type null }` in ~/.asoundrc) is enough to exercise the
//! real callback, conversion and recording path.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use uta_audio::device::DeviceSelection;
use uta_audio::wav::wav_info;
use uta_audio::{AudioEngine, BackendChoice};

#[test]
fn device_capture_and_callbacks() {
    if std::env::var("UTA_DEVICE_TESTS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set UTA_DEVICE_TESTS=1 to run");
        return;
    }
    let sel = DeviceSelection { input_enabled: true, ..Default::default() };
    let mut eng = AudioEngine::start(BackendChoice::Device(sel)).expect("open default device");
    let info = eng.info().clone();
    eprintln!("opened {info:?}");
    let sr = info.sample_rate as u64;
    let start = Instant::now();
    while eng.shared().callbacks.load(Ordering::Relaxed) < 10 && start.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(eng.shared().callbacks.load(Ordering::Relaxed) >= 10, "output callbacks are running");
    if eng.has_input() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("dev.wav");
        eng.capture(p.clone(), 0, None).unwrap();
        std::thread::sleep(Duration::from_millis(600));
        let out = eng.stop_recording().unwrap();
        let wi = wav_info(&p).unwrap();
        eprintln!("recorded {} frames, status {:?}", wi.frames, eng.status());
        assert_eq!(wi.frames, out.result.files[0].frames);
        assert!(wi.frames > sr / 4, "captured audio from the device");
        assert_eq!(wi.sample_rate as u64, sr);
    }
    assert!(!eng.status().device_lost);
}
