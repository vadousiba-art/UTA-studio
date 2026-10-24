# Project status

_Last updated: 2026-10-24_

## Current milestone

**M7 — polish, Windows CI, packaging.** M0–M6 are implemented.

## Completed

* M0 foundation: workspace, Tauri shell, UI system, settings, docs.
* M1 recording: devices, engine, meters, WAV recording/playback, latency
  compensation, waveform peaks.
* M2 analyzer: pitch/notes/vibrato/dynamics/spectral/onsets, quality gating,
  comparisons, Analyzer screen.
* M3 studio: projects with revisions/autosave/recovery, multitrack
  recording, takes/comping, clip editing, undo/redo, import, export.
* M4 effects: six built-in effects, chains, presets.
* M5 VST3 hosting: isolated helper, scanning, real-time bridge with delay
  compensation, generic parameters, state in projects, crash isolation and
  reload, offline export, Plugin Manager UI.
* M6: Coach (curriculum, exercises, evaluation, adaptation, review,
  health rules, Coach UI) and Song Lab (songs, backing/reference WAV
  import, LRC/text lyrics with readings, romaji and mora counts, tap
  timing, phrases, pitch-preserving slow-down renders, seamless phrase
  loops, attempts recorded with backing and compared with the reference,
  notes, deletion).

## In progress

* M7 tasks (see Next steps).

## Blocked / not verifiable here

* Windows runtime verification — the development environment is Linux
  without Windows; needs the CI workflow (M7) and a manual run on Windows.
* Real audio hardware — no sound card in the development container.

## Known bugs

* The packaged (`tauri build --debug --no-bundle`) binary showed a blank
  window once under Xvfb while `tauri dev` worked; not re-checked yet.
* Under the ALSA null device the status bar shows large "dropout" counts
  (the null device is unpaced); expected there, not on real hardware.

## Last successful runs (Linux, 2026-10-24)

| Check | Result |
| --- | --- |
| `cargo test --workspace` | 159 passed, 0 failed |
| `cargo clippy --workspace --all-targets` | no warnings |
| `npm test` | 2 passed |
| `npm run typecheck` | clean |
| `npm run sidecar` | helper built (release) |
| `tauri dev` (Xvfb) | app ran; Plugin Manager scan, plugin insert, parameter drag, crash + reload; Song Lab create, WAV/LRC import through the file dialog, phrase, 75 % render, loop playback, attempts with result view — verified by screenshots |

## Next steps (in order)

1. `.github/workflows/windows.yml`: cargo test, clippy, npm test/typecheck,
   `tauri build` (NSIS + MSI) with the plugin helper sidecar, upload
   installers as artifacts.
2. Re-check the packaged build (blank window seen once under Xvfb).
3. Re-verify the Coach lesson flow in the running app.
4. Measure CPU/RAM (idle, playback with effects, plugin, analysis) and
   document.
5. Accessibility pass (focus order, labels, contrast); docs sync.
