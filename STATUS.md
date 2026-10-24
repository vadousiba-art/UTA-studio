# Project status

_Last updated: 2026-10-24_

## Current milestone

**M6 — Song Lab** (Coach part of M6 is done). M0–M5 are implemented.

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
* Coach: curriculum, exercises, evaluation, adaptation, review, health
  rules, Coach UI.

## In progress

* Song Lab backend and UI (the Song Lab screen currently shows an
  "under construction" notice; no fake controls).

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
| `cargo test --workspace` | 148 passed, 0 failed |
| `cargo clippy --workspace --all-targets` | no warnings |
| `npm test` | 2 passed |
| `npm run typecheck` | clean |
| `npm run sidecar` | helper built (release) |
| `tauri dev` (Xvfb) | app ran; Plugin Manager scan, plugin insert, parameter drag, crash + reload verified by screenshots |

## Next steps (in order)

1. Song Lab backend in `uta-core` (`songlab.rs`): song CRUD in the
   library `songs` table; backing/reference import into
   `data/songs/<id>/`; lyrics parsing (plain text, LRC) with kana→romaji;
   speed renders via `uta_dsp::stretch` as jobs (cached); phrase loop
   playback; attempt recording + analysis + DTW comparison vs. reference;
   notes. Headless tests.
2. Song Lab UI replacing `src/workspaces/SongLab.tsx`.
3. Re-verify the Coach lesson flow in the running app.
4. M7: `.github/workflows/windows.yml` (cargo test, npm test, tauri build
   NSIS/MSI, upload artifacts); re-check packaged build; measure CPU/RAM;
   accessibility pass; docs sync.
