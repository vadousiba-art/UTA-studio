# Known limitations

Honest list of what does not work, or has not been verified, in this
version.

## Platform and verification

* **Not yet verified on Windows.** All development and automated tests ran
  on Linux. The code uses only cross-platform APIs (cpal/WASAPI, Tauri,
  WebView2) and Windows-specific paths are implemented, but the Windows
  build, installer, audio devices and plugin loading still need a Windows
  run (CI workflow + manual test).
* **No audio hardware in the development environment.** Recording and
  playback were verified with the engine's null backend and ALSA's null
  device. Real latency, dropout behaviour and microphone quality are
  untested.
* The packaged (custom-protocol) build showed a blank window once under the
  Linux virtual display used for testing while `tauri dev` rendered
  correctly; this has not been re-checked yet.

## Audio

* **No ASIO** (licence incompatible). Windows audio uses WASAPI shared mode
  through cpal; expect 10–30 ms device latency. Recording latency is
  compensated automatically plus a manual offset in Settings.
* The project sample rate must match the device rate to play or record (a
  clear message explains how to change it). Imported files are converted.
* Import: **WAV only** (16/24/32-bit integer and 32-bit float, any rate).
  MP3/FLAC/AAC/OGG import is not implemented.
* Export: WAV only (16/24-bit integer with optional dither, 32-bit float).
* No MIDI, no automation lanes, no time-stretching of clips in the Studio,
  no pitch correction.

## Plugins

* VST3 **effects only**, 64-bit. No VST2, CLAP, AU, LV2, instruments, MIDI
  effects or sidechains.
* **No plugin editor windows** — parameters are shown as generic sliders.
  Plugins whose important settings are only reachable through their GUI
  can't be fully configured here.
* Each plugin adds **1024 frames (~21 ms at 48 kHz)** of bridge latency
  plus its own latency. Playback and export are compensated; monitoring the
  live input through plugins is delayed.
* Each plugin uses one helper process (a few MB of RAM plus the plugin's
  own memory).
* Tested only with the project's own test plugin.

## Coach and analysis

* Only the deterministic coach (Layer A) exists. Optional local LLM
  explanations (Layer B) and local audio models (Layer C) are not
  implemented; nothing requires them.
* Analysis accuracy was validated on synthetic signals; real voices with
  breathiness, heavy vibrato or strong reverb may be graded Inconclusive
  more often. Background noise and a noisy room reduce reliability.
* Tone indicators (spectral centroid, H1–H2, periodicity) are descriptive
  hints, not diagnoses.
* The app cannot detect vocal strain or injury; it relies on self-reports.

## Song Lab

* In progress — see STATUS.md for exactly which parts work.
* Kanji readings are not generated automatically (no dictionary is
  bundled); romaji is generated from kana, and the user types readings for
  kanji lines.

## UI

* English UI only (Japanese text is supported in lyrics, names and notes).
* Keyboard shortcuts cover navigation and core Studio actions; not every
  control has one. Screen-reader support has had only basic checks
  (labels, roles), no full audit.
