# Testing

## Commands

```bash
cargo test --workspace              # all Rust unit + integration tests
cargo test -p uta-core              # app workflows only
cargo clippy --workspace --all-targets
npm test                            # frontend (vitest)
npm run typecheck                   # TypeScript strict
UTA_DEVICE_TESTS=1 cargo test -p uta-audio --test device_smoke   # needs real audio hardware
```

Plugin tests need `bash` and build the fixture plugin with
`scripts/build-test-plugin.sh` (first run takes about a minute).

## What is covered

| Area | Where | How |
| --- | --- | --- |
| DSP | `crates/uta-dsp` | Unit tests per effect: frequency responses (EQ bands, HPF), compressor gain reduction and makeup, gate attenuation, reverb/delay energy and tails, smoothing convergence, pan law, tone generators, WSOLA duration and pitch preservation. |
| Analysis | `crates/uta-analysis` | Synthetic signals with known truth: sine/harmonic tones within a few cents, glides frame by frame, vibrato rate/extent, drift and wobble, octave-error folding, silence/noise rejection, note segmentation, dynamics swells, quality gating (clipping, too quiet, too short), target comparison, DTW contour comparison, onset detection. |
| Coach | `crates/uta-coach` | Exercise generation inside narrow and wide ranges at every level, evaluation of synthetic "singers" (accurate → Success, off-pitch → Needs work, noise → Inconclusive), progression rules, spaced review, health rules (rest, professional recommendation, texture lock). |
| Projects | `crates/uta-project` | Edit operations and validation, undo/redo, takes/comping/flatten, revisions, checksum corruption fallback, autosave recovery, format migration, path-escape rejection, plugin slot round trip, library tables. |
| Audio engine | `crates/uta-audio` | With the null backend: playback of real WAV files through the streamer, recording with exact length, loops, latency compensation, metronome/sequence timing, preview, graph mixing/meters, plugin delay compensation, offline render vs. expected samples, export at other sample rates and bit depths, import with resampling, waveform peaks. |
| Plugin hosting | `crates/uta-plugin-host`, `crates/uta-plugins`, `crates/uta-core/tests/plugins.rs` | Real helper process + fixture VST3: scan, load, process, parameter text, state round trip across processes, bad/missing bundles, crash isolation, sample-aligned real-time bridge, manager status (running/crashed/missing/disabled/unavailable), reload, offline export through the plugin, project save/reopen with state, undo of parameter edits. |
| Song Lab | `crates/uta-core/tests/songlab.rs`, `songlab.rs`/`romaji.rs` unit tests | Import with conversion, LRC/plain lyrics parsing, romaji and mora counts, readings, invalid phrases rejected, pitch-preserving 75 % render (duration and pitch measured), looped phrase playback, attempt with backing compared with the reference (pitch within 30 ¢), silent attempt not scored, deletion of files and attempts, path-safety of song ids. |
| App workflows | `crates/uta-core/tests/workflow.rs` | `AppCore` with a synthetic singer as microphone: mic off by default, analyzer record/analyse/compare, coach attempt graded and scheduled, range check, studio record → takes → comp → edit → undo/redo → save → reopen → export → decode, autosave recovery after a crash, import + missing asset relink, engine restart keeping the project. |
| Frontend | `src/**/*.test.ts` | Music helpers (note names, formatting). |

## Manual verification done during development

The real application was run (Linux, `tauri dev` under a virtual X server)
and these screens were exercised by clicking: first run, Home, Analyzer,
Studio start, Studio project with mixer, Plugin Manager scan, inserting a
VST3 plugin, dragging its parameter, crashing it (test parameter) and
reloading it; Song Lab: creating a song, importing backing and reference
WAVs through the file dialog, importing an LRC file, making a phrase,
preparing 75 % speed, looped playback, recording attempts and viewing the
result with the reference contour. Screenshots were inspected. This environment has **no sound
card**, so audible output and real microphone input were not verified; the
engine ran against ALSA's null device. See STATUS.md for the last runs.

## Not yet covered

* Windows-specific runtime behaviour (WASAPI devices, WebView2, installer,
  plugin bundles in `Contents\x86_64-win`) — to be covered by the Windows CI
  workflow and manual testing on a Windows machine.
* Real microphones and real singers (accuracy figures come from synthetic
  signals).
* Commercial third-party VST3 plugins.
* UI component tests beyond helpers.
