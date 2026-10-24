# Third-party plugin hosting (VST3)

## What is supported

* 64-bit **VST3 audio effects** (`.vst3` bundles or single-file modules) on
  Windows (`Contents\x86_64-win\*.vst3`) and Linux
  (`Contents/x86_64-linux/*.so`).
* Scanning the standard VST3 folders plus user-added folders.
* Inserting plugins into any track or the master chain, bypass, reorder,
  remove, undo/redo.
* Generic parameter control (every parameter the plugin exposes, with the
  plugin's own value text), undoable.
* Plugin state saved in the project (component + controller state) and
  restored when the project is opened, including on another computer.
* Plugin latency reporting and compensation, offline (export) rendering.
* Crash and hang isolation with recovery.

Not supported (and never claimed in the UI): VST2, CLAP, AU, LV2, 32-bit
plugins, instruments/MIDI effects, sidechain inputs, plugin editor windows
(GUIs), automation lanes. See [known-limitations.md](known-limitations.md).

## Process isolation

Every loaded plugin runs in its own `uta-plugin-host` process. The app talks
to it over stdin/stdout using length-prefixed frames
(`uta-plugin-protocol`): JSON control messages (`Hello`, `Scan`, `Load`,
`GetParams`, `SetParams`, `GetState`, `SetState`, `Shutdown`) and binary
audio blocks (parameter changes + left/right samples).

Why separate processes: a plugin is foreign native code. If it crashes,
leaks or hangs inside the app, the whole session (and unsaved work) is gone.
In a helper, the worst case is that one effect slot stops processing.

* **Scanning** loads each bundle in a throwaway helper with a 30 s timeout.
  A bundle that crashes or hangs is listed under "Could not be used" with the
  reason; the scan continues. Results are cached by path and modification
  time, so unchanged bundles are not loaded again.
* **Loading** (45 s timeout) instantiates the effect class, connects the
  edit controller, negotiates a stereo (or mono) main bus, sets up
  processing for the session's sample rate and a 512-frame maximum block,
  applies saved state, then saved parameter values.
* **Every request** has a timeout (audio blocks: 2 s). On timeout or broken
  pipe, the helper is killed and the slot is marked *crashed*.
* The helper sanitises plugin output (NaN/∞ → 0) before returning it.
* On Windows the helper is started with `CREATE_NO_WINDOW`.

## Real-time bridge

The audio callback must never wait for another process. Each plugin slot in
the live graph is an `RtEndpoint`:

1. The callback copies its input block into a lock-free ring (`rtrb`) and
   reads processed audio from a second ring.
2. A worker thread takes whatever input is available (≤ 512 frames), sends it
   to the helper, and pushes the reply into the output ring.
3. The output ring starts primed with **1024 frames** of silence. This is the
   bridge latency (≈ 21 ms at 48 kHz) and it is reported to the graph, which
   delays all other tracks by the same amount (plugin delay compensation), so
   playback stays sample-aligned. The plugin's own reported latency is added.
4. If the helper is late, the endpoint outputs the equally delayed dry signal
   for the missing frames and later discards the same number of late frames,
   so the plugin output never drifts. Late blocks are counted per slot.
5. If the helper crashes or hangs, the endpoint keeps outputting the delayed
   dry signal (no gap, no shift) and the slot shows **crashed** with a
   **Reload** button.
6. Bypassed plugins keep running (tails and state stay continuous) but the
   delayed dry signal is output, so bypass never shifts timing.

Recording compensation includes the graph latency, so takes recorded while
listening to plugin-processed playback line up. Monitoring the live input
through a plugin is delayed by the bridge latency; the app recommends
recording dry and adding plugins afterwards.

**Export** does not use the bridge: separate helpers are started in offline
mode and each block is processed synchronously, with the current plugin
state. Exports are trimmed by the graph latency so they line up with the
timeline. Problems (missing plugin, crash during export) are listed as
warnings in the export report; the affected slot is exported unprocessed.

## State and missing plugins

* On save, autosave, and before removing a slot or track, the app reads each
  running plugin's state (`vst3:<component hex>[:<controller hex>]`) into the
  project. Parameters changed in the app are also stored by id and applied
  on top of the state when loading.
* A project that references a plugin that is not installed keeps the slot,
  its state and parameters untouched. The slot passes audio through and
  shows **missing**. If the same plugin (same class id) is installed in a
  different place, it is found through the scan cache automatically, or the
  user can relink it.
* Disabled bundles (Plugin Manager switch) are unloaded; their slots pass
  audio through and keep their settings.

## Bundling

`scripts/prepare-sidecar.mjs` (run by `npm run sidecar`, and automatically
by `tauri dev`/`tauri build`) builds `uta-plugin-host` in release mode and
copies it to `src-tauri/binaries/uta-plugin-host-<target-triple>[.exe]`.
Tauri's `externalBin` places it next to the app executable, where
`uta_plugins::locate_helper()` finds it (`UTA_PLUGIN_HOST` overrides). If the
helper is missing, the Plugin Manager says so and built-in effects keep
working.

## Licensing

The VST3 bindings come from the `vst3` crate (MIT/Apache-2.0), generated
from the VST3 SDK headers, which Steinberg has published under the MIT
licence since SDK 3.8. No Steinberg SDK code or third-party plugin binaries
are shipped. The test plugin in `fixtures/test-plugins/uta-test-gain` is our
own and only used by tests. "VST" is a trademark of Steinberg Media
Technologies GmbH.

## Testing

`crates/uta-plugin-host/tests/helper.rs`, `crates/uta-plugins/tests/bridge.rs`
and `crates/uta-core/tests/plugins.rs` build the fixture plugin and the real
helper, then verify scanning, loading, processing, parameter text,
state round-trips across processes, sample alignment of the real-time
bridge, crash isolation and reload, missing/disabled/unavailable handling,
offline export through a plugin, and project save/reopen with plugin state.
These run on Linux in development; Windows coverage comes from the CI
workflow. No third-party commercial plugin has been tested yet.
