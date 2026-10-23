//! Print the audio devices cpal can see: `cargo run -p uta-audio --example devices`.
fn main() {
    match uta_audio::device::list_devices() {
        Ok(list) => println!("{}", serde_json::to_string_pretty(&list).unwrap_or_default()),
        Err(e) => eprintln!("device enumeration failed: {e}"),
    }
    let sel = uta_audio::device::DeviceSelection { input_enabled: true, ..Default::default() };
    match uta_audio::AudioEngine::start(uta_audio::BackendChoice::Device(sel)) {
        Ok(e) => println!("opened: {:?}", e.info()),
        Err(e) => println!("could not open default device: {e}"),
    }
}
