# Dependency and licence inventory

Everything UTA Vocal Lab uses is free and open source. Nothing is paid, and
nothing requires an account or network service. The app itself is MIT
licensed (see `LICENSE`).

## Rust (direct dependencies)

| Crate | Version | Licence | Used for |
| --- | --- | --- | --- |
| tauri, tauri-build | 2.12 / 2.7 | Apache-2.0 OR MIT | Desktop shell, IPC |
| tauri-plugin-dialog | 2.8 | Apache-2.0 OR MIT | File/folder pickers |
| tauri-plugin-opener | 2.7 | Apache-2.0 OR MIT | Reveal files in Explorer |
| cpal | 0.18 | Apache-2.0 | Audio device I/O (WASAPI on Windows, ALSA on Linux) |
| hound | 3.5 | Apache-2.0 | WAV read/write |
| rubato | 5.0 | MIT OR Apache-2.0 | Sample-rate conversion |
| realfft | 3.5 | MIT | FFT (pitch detection, spectral analysis) |
| rtrb | 0.3 | MIT OR Apache-2.0 | Lock-free ring buffers (real-time audio) |
| rusqlite (bundled SQLite) | 0.40 | MIT (SQLite: public domain) | Library and project databases |
| vst3 | 0.3 | MIT OR Apache-2.0 | VST3 COM bindings (helper process only) |
| libloading | 0.9 | ISC | Loading plugin modules (helper process only) |
| crossbeam-channel | 0.5 | MIT OR Apache-2.0 | Thread messaging |
| parking_lot | 0.12 | MIT OR Apache-2.0 | Mutexes |
| serde, serde_json | 1.0 | MIT OR Apache-2.0 | Serialisation |
| uuid | 1 | Apache-2.0 OR MIT | Ids |
| chrono | 0.4 | MIT OR Apache-2.0 | Dates |
| dirs | 7 | MIT OR Apache-2.0 | Standard folders |
| thiserror | 2 | MIT OR Apache-2.0 | Error types |
| tracing, tracing-subscriber | 0.1 / 0.3 | MIT | Logging |
| tempfile (tests only) | 3 | MIT OR Apache-2.0 | Test folders |

Transitive Rust dependencies (about 520 crates, mostly via Tauri) are under
permissive licences: MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0,
Unlicense, CC0. A few are **MPL-2.0** (`cssparser`, `cssparser-macros`,
`selectors`, `dtoa-short` — used by Tauri's HTML handling — and
`option-ext` via `dirs`); MPL-2.0 is a file-level licence that permits use
in MIT-licensed applications as long as those files' source stays
available (they are unmodified crates.io releases). `r-efi` is offered
under MIT/Apache-2.0 (LGPL is one alternative, not a requirement). No GPL
code is linked.

Regenerate this summary with:

```bash
cargo metadata --format-version 1 | python3 -c "import json,sys,collections; d=json.load(sys.stdin); print(collections.Counter(p.get('license') for p in d['packages']))"
```

## JavaScript

| Package | Licence | Used for |
| --- | --- | --- |
| react, react-dom 19 | MIT | UI |
| zustand 5 | MIT | UI state |
| lucide-react | ISC | Icons |
| @tauri-apps/api, plugin-dialog, plugin-opener | Apache-2.0 OR MIT | IPC bindings |
| vite 8, @vitejs/plugin-react (dev) | MIT | Build |
| typescript 6 (dev) | Apache-2.0 | Type checking |
| vitest, jsdom, @testing-library/* (dev) | MIT | Tests |
| @tauri-apps/cli (dev) | Apache-2.0 OR MIT | Build/bundle |

## Platform components

* **Microsoft Edge WebView2 Runtime** (Windows) — redistributable, free;
  preinstalled on current Windows 10/11; the installer can fetch the
  bootstrapper if it is missing.
* **WASAPI** (Windows audio) through cpal. **ASIO is not included**: the
  Steinberg ASIO SDK licence (GPLv3 or a proprietary agreement) is not
  compatible with this distribution model, so low-latency ASIO drivers are
  not used. See known limitations.

## Specifications and trademarks

* VST3 interfaces: Steinberg VST3 SDK headers are MIT-licensed since SDK 3.8
  (the `vst3` crate's bindings are generated from them). "VST" is a
  registered trademark of Steinberg Media Technologies GmbH. No Steinberg
  code or third-party plugin binaries are distributed.
* The test plugin under `fixtures/test-plugins/uta-test-gain` is original
  code (based on the `vst3` crate's MIT/Apache-2.0 gain example) and is only
  used by tests; it is not shipped.

## Content

No songs, lyrics, backing tracks, vocal recordings, sample libraries or
fonts with restrictive licences are bundled. The UI uses the system font
stack. Users import their own legally obtained material.
