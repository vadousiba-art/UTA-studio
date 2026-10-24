# Task checklist

Milestones were built in order M0 → M7. `[x]` = implemented and tested,
`[~]` = partially done, `[ ]` = not started. See STATUS.md for details.

## M0 Foundation
- [x] Cargo workspace, Tauri 2 shell, React/TS/Vite frontend, strict TS
- [x] Design system (tokens, components), navigation, status bar, shortcuts
- [x] Settings persistence, data folders, logging, error boundary
- [x] Documentation set (README, architecture, product spec, training method,
      plugin hosting, testing, known limitations, dependencies, tasks, status)
- [ ] Windows CI build (moved to M7)

## M1 Recording foundation
- [x] Device enumeration and selection (cpal), sample rate / buffer choice
- [x] Real-time engine with lock-free rings, null backend for tests
- [x] Input metering with clip indicators, microphone off by default
- [x] Recording to WAV (crash-tolerant header flushing), playback, preview
- [x] Waveform peaks with cache; latency measurement and compensation

## M2 Vocal analyzer
- [x] YIN pitch with confidence, octave-error correction, live pitch
- [x] Notes, sustain, vibrato, dynamics, spectral indicators, onsets
- [x] Quality gating (Inconclusive instead of guessing)
- [x] Target comparison and DTW contour comparison
- [x] Analyzer UI with recording library

## M3 Multitrack studio
- [x] Project model, edits, undo/redo, revisions, autosave, recovery
- [x] Recording on armed tracks, takes and comping, flatten
- [x] Timeline editing (move, trim, split, duplicate, fades, gain)
- [x] Loop, markers, metronome/count-in, tempo
- [x] WAV import with conversion, missing-asset relink
- [x] Export with verification report

## M4 Built-in effects
- [x] EQ, compressor, gate, reverb, delay, utility; smoothing; metering
- [x] Effect chains per track and master, bypass, reorder
- [x] Presets (save/load/delete), EQ curve display

## M5 VST3 hosting
- [x] Isolated helper process, protocol, test fixture plugin
- [x] Scanning with cache and failure reporting, enable/disable
- [x] Real-time bridge with latency compensation and dry fallback
- [x] Parameters (generic UI), state in projects, missing-plugin handling
- [x] Crash isolation and reload; offline export through plugins
- [x] Plugin Manager UI, mixer integration
- [ ] Verification on Windows with real third-party plugins

## M6 Coach + Song Lab
- [x] Curriculum, exercise generation, evaluation, progression, review,
      health rules, coach UI (lessons, range check, skill map, library)
- [x] Song Lab: songs, backing/reference import, lyrics with kana/romaji,
      timing, phrase loops at reduced speed, attempts and comparison, notes
- [ ] Visual re-verification of the Coach lesson flow in the running app

## M7 Polish and release
- [ ] Windows GitHub Actions workflow (tests + NSIS/MSI bundles)
- [ ] Re-check the packaged build (blank window seen once under Xvfb)
- [ ] Performance measurement (CPU/RAM) documented
- [ ] Accessibility pass (keyboard focus order, labels)
- [ ] Final docs sync
