# UTA Vocal Lab

A free, local-first desktop app for learning to sing and producing vocals:

- **Coach** — an adaptive, rule-based singing trainer (six-stage curriculum,
  transparent progression rules, spaced review, built-in vocal-health rules).
- **Analyzer** — record a phrase and get measured feedback: pitch accuracy,
  stability, vibrato, onsets, dynamics, tone indicators, with quality checks
  that refuse to grade bad recordings.
- **Song Lab** — practise Japanese songs you legally own: backing track,
  lyrics with kana/romaji, phrase loops, slowed-down practice, attempts and
  comparisons. *(in progress — see [STATUS.md](STATUS.md))*
- **Recording Studio** — real multitrack recording with takes and comping,
  clip editing, undo/redo, autosave/recovery, built-in effects (EQ,
  compressor, gate, reverb, delay, utility) with presets, WAV import and
  export.
- **Plugins** — hosting of third-party 64-bit **VST3 effects**, each in its
  own isolated helper process, with scanning, enable/disable, generic
  parameter controls, state saved in projects and crash recovery.
- **Progress & Library** — practice history, skill levels, recordings, notes,
  backups.

Everything runs offline on the computer. There is no account, no cloud
service and no audio upload. The microphone stays off until you turn it on.

> UTA Vocal Lab is a practice tool. It is not a substitute for a qualified
> voice teacher or a medical professional. If singing hurts, stop and rest;
> if discomfort or hoarseness persists, see a doctor or speech-language
> pathologist.

Target platform: **Windows 10/11 64-bit**, 8 GB RAM, no GPU required. The
code also builds and runs on Linux (used for development and automated
tests).

## Status

See [STATUS.md](STATUS.md) for exactly what works, what is in progress, and
the results of the last build and test runs. Known limitations are listed in
[docs/known-limitations.md](docs/known-limitations.md).

## Building on Windows

Prerequisites (all free):

1. [Rust](https://rustup.rs) (stable, MSVC toolchain, 1.90 or newer).
2. [Node.js](https://nodejs.org) 20 or newer.
3. Microsoft C++ Build Tools (installed with Visual Studio Build Tools,
   "Desktop development with C++").
4. WebView2 Runtime (already present on up-to-date Windows 10/11).

```powershell
git clone <this repository>
cd UTA-studio
npm ci
npm run tauri dev        # development build with hot reload
npm run tauri build      # installers in target/release/bundle (NSIS + MSI)
```

`tauri dev` / `tauri build` automatically run `npm run sidecar`, which builds
the plugin helper (`uta-plugin-host`) and places it in `src-tauri/binaries/`
so it is bundled next to the app.

The installer is per-user (no administrator rights needed). If the WebView2
runtime is missing, the installer downloads Microsoft's bootstrapper.

## Building on Linux (development)

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libasound2-dev \
  librsvg2-dev libayatana-appindicator3-dev build-essential
npm ci
npm run tauri dev
```

## Tests

```bash
cargo test --workspace     # Rust: DSP, analysis, coach, engine, projects, plugins, workflows
npm test                   # frontend unit tests
npm run typecheck
cargo clippy --workspace --all-targets
```

The plugin tests build a small test VST3 effect (`fixtures/test-plugins`)
and the real helper process, then load, process, save state, crash and
reload it. Device tests that need real audio hardware are opt-in
(`UTA_DEVICE_TESTS=1`). See [docs/testing.md](docs/testing.md).

## Documentation

| Document | Contents |
| --- | --- |
| [docs/product-spec.md](docs/product-spec.md) | What the app does, screen by screen |
| [docs/architecture.md](docs/architecture.md) | Crates, threads, audio engine, data storage |
| [docs/training-method.md](docs/training-method.md) | Curriculum, lesson loop, adaptation and health rules |
| [docs/plugin-hosting.md](docs/plugin-hosting.md) | VST3 hosting design, isolation, latency, state |
| [docs/testing.md](docs/testing.md) | Test strategy and how to run each suite |
| [docs/known-limitations.md](docs/known-limitations.md) | What does not work (yet) |
| [docs/dependencies.md](docs/dependencies.md) | Dependency and licence inventory |
| [docs/tasks.md](docs/tasks.md) | Milestone task checklist |
| [STATUS.md](STATUS.md) | Current state, last build/test results, next steps |

## Privacy

- No account, no telemetry, no network access is required or used at run
  time (the Windows installer may download the WebView2 runtime if missing).
- Recordings, projects and the practice database stay in your folders:
  `%APPDATA%\UTA Vocal Lab` (library, recordings) and
  `Documents\UTA Vocal Lab\Projects` (projects).
- The microphone is opened only after you enable it; the status bar always
  shows the active input and when recording is running.
- Settings has controls to delete practice history and to back up the
  library. Recordings can be deleted individually.
- The app never downloads or installs plugins.

## Content and licences

No copyrighted songs, lyrics, backing tracks or vocal recordings are
bundled. Import only material you have the right to use. The app's own code
is MIT-licensed; third-party components are listed in
[docs/dependencies.md](docs/dependencies.md).
