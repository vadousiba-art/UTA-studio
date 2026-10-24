# Architecture

UTA Vocal Lab is a Tauri 2 desktop app: a React/TypeScript UI in the system
WebView, and a Rust backend that does all audio, analysis, coaching and
storage. The UI never touches audio directly; it calls Rust commands over
Tauri IPC and polls compact status snapshots.

```
┌──────────────────────────── UI (React, TypeScript, Vite) ────────────────────────────┐
│ Home · Analyzer · Coach · Song Lab · Studio · Plugins · Progress · Settings         │
│ src/api/client.ts  ── invoke() ──►  src-tauri/src/commands.rs (thin, async)          │
└──────────────────────────────────────────────────────────────────────────────────────┘
                                         │ spawn_blocking
┌──────────────────────────── uta-core (AppCore) ──────────────────────────────────────┐
│ settings · analyzer · coach · studio sessions · plugins · presets · jobs · live pitch │
└──────┬──────────────┬──────────────┬──────────────┬──────────────┬────────────────────┘
       │              │              │              │              │
   uta-audio     uta-analysis    uta-coach     uta-project     uta-plugins ──► uta-plugin-host
 (engine, I/O,  (pitch, notes,  (curriculum,  (model, edits,  (client, bridge,   (separate process,
  graph, render) vibrato, …)     rules)        SQLite stores)  scan, manager)     one per plugin)
       │
    uta-dsp (effects, smoothing, metering, tones, time-stretch)
```

## Crates

| Crate | Responsibility |
| --- | --- |
| `uta-dsp` | Pure DSP: biquads, EQ, compressor, gate, Freeverb-style reverb, delay, utility, metering, parameter smoothing, tone/click generators, WSOLA time-stretch. No allocation in `process`. |
| `uta-analysis` | Offline and live vocal analysis: FFT-based YIN pitch with confidence, octave-error correction, note segmentation, sustain stability, vibrato, dynamics, spectral indicators, recording quality checks, target comparison, DTW contour comparison, onset detection. |
| `uta-project` | Project document model (tracks, clips, take groups, comps, effects, plugin slots), validated edit operations, snapshot undo/redo, the per-project SQLite store with revisions, and the app library database. |
| `uta-audio` | Real-time engine (CPAL), device enumeration, lock-free command/garbage rings, streaming playback, recording to WAV, the mixing graph (shared by live playback and offline render), latency compensation, WAV import/resampling, waveform peaks, export. |
| `uta-coach` | Deterministic coach ("Layer A"): skills and prerequisites, exercise generation in the user's range, evaluation, progression, spaced review, health rules. |
| `uta-plugin-protocol` | Framed stdin/stdout protocol between the app and the plugin helper. |
| `uta-plugin-host` | The helper executable: loads one VST3 module and one effect instance, processes audio blocks, reads/writes parameters and state. |
| `uta-plugins` | App side of plugin hosting: spawning helpers with timeouts, scanning, the real-time bridge, offline processors, the per-slot instance manager. |
| `uta-core` | `AppCore`: owns the engine, library, settings, open project, plugin manager and job pool; exposes every user operation as a method. Fully usable headlessly — the workflow tests drive it with a synthetic "singer". |
| `src-tauri` | Tauri shell: window, IPC command wrappers, autosave timer, logging. |

## Audio engine

* **Callback** (`EngineCore::process`, audio thread): no allocation, locks,
  I/O or blocking. It reads commands from a bounded `rtrb` ring, mixes the
  graph, writes recorded input to a recorder ring, feeds the analysis ring,
  generates guide tones/clicks/metronome, and plays previews. Objects it
  replaces (old graphs, sequences, streams) go back through a garbage ring
  and are dropped on a control thread.
* **Input**: a separate CPAL input stream writes into a ring the output
  callback reads (drift is handled by bounded dropping/padding and counted as
  dropouts in the status bar).
* **Streamer thread**: reads clips from disk into per-track rings ahead of
  the playhead (0.4 s prefill, 2 s buffers), handles loops and relocation by
  generation counters.
* **Recorder thread**: writes 32-bit float WAV, flushing the header about
  once per second so a crash loses at most ~1 s.
* **Graph** (`graph.rs`): per-track chain → delay compensation → gain/pan →
  meters → master chain → master gain. The same code runs live and in the
  offline renderer, so exports sound like playback. Chains contain built-in
  effects or plugin processors behind the `ChainProcessor` trait.
* **Latency**: recordings are shifted by the measured device latency
  (from stream timestamps, or buffer size), the user's manual offset, and the
  current graph latency (plugins).
* **Null backend**: a device-free backend with an input generator and output
  sink, used by tests and as a silent fallback when no device can be opened.

## Plugin hosting

See [plugin-hosting.md](plugin-hosting.md). Summary: one helper process per
plugin instance; the audio thread only touches lock-free rings; a worker
thread exchanges blocks with the helper; the bridge adds a fixed, reported
latency that the graph compensates; crashes fall back to the dry signal.

## Data storage

| Data | Where | Format |
| --- | --- | --- |
| Library (settings, recordings index, attempts, skill levels, reviews, songs, presets, plugin scan cache, notes) | `%APPDATA%\UTA Vocal Lab\library.sqlite` | SQLite with `user_version` migrations |
| Practice recordings | `%APPDATA%\UTA Vocal Lab\recordings` | WAV + cached `.analysis.json` |
| Projects | `Documents\UTA Vocal Lab\Projects\<name>.utaproj\` | folder: `project.utadb` (SQLite revisions) + `audio/` + `cache/` |
| Backups | `%APPDATA%\UTA Vocal Lab\backups` | SQLite copies (`VACUUM INTO`) |
| Logs | `%APPDATA%\UTA Vocal Lab\logs` | text |

Each project save writes a new **revision** (JSON document, format version,
FNV-1a checksum). The last 30 saves and 20 autosaves are kept. Opening a
project loads the newest valid manual save, skipping damaged revisions, and
offers a newer autosave for recovery. Asset paths are relative and validated
so a project cannot reference files outside its folder. Older document
formats are migrated on load.

## Concurrency inside AppCore

`AppCore` methods are called from Tauri's blocking pool. State is guarded by
`parking_lot` mutexes with a fixed lock order to prevent deadlocks:
`studio → audio → plugins → capture → library → settings`. Long work
(import, export, plugin scans, analysis) runs as cancellable **jobs** with
progress, isolated with `catch_unwind`.

## UI

React 19 + Zustand. Each workspace is a component under `src/workspaces`.
Engine status (meters, position, dropouts, DSP load) is polled every 40 ms;
live pitch every frame batch. Studio edits are sent as typed `EditOp`s; drags
send throttled *transient* edits and one final edit, which forms a single
undo step.
