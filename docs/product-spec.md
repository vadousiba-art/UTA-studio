# Product specification

**User:** a beginner singer (initially a 17-year-old who wants to sing and
produce Japanese music in expressive styles such as Ado's), practising alone
at home on a Windows 10 laptop with 8 GB RAM, no GPU, sometimes offline,
with a USB or built-in microphone and headphones.

**Promise:** short, safe, measurable practice that adapts to the singer, a
place to work on real songs, and a studio to record and mix them — all
free, offline and private.

## Principles

* Measured feedback, never invented praise. If a recording can't be graded
  reliably, say so.
* Technique, not imitation: build transferable skills; never reward loudness
  or high notes for their own sake.
* Vocal health first: discomfort stops the exercise; rest and professional
  advice are recommended when needed.
* Every visible control works. Unsupported things are absent or clearly
  labelled, never faked.
* Local-first: no account, no cloud, no uploads, microphone off by default.

## Navigation

Left rail: Home · Analyzer · Coach · Song Lab · Studio · Plugins · Progress ·
Settings (`Ctrl+1…7`). The status bar always shows the output device and
sample rate, the active input (or "microphone off"), recording state,
dropout count, DSP load and latency.

## Screens

### First run
Explains privacy, asks whether to enable the microphone (off by default),
links to Settings for device choice, and shows the health disclaimer.

### Home
Recommended next exercise (with the reason), recent projects, recent
recordings, skill progress, practice days, reviews due, comfortable range.

### Analyzer
Record a phrase (with live pitch display and input meter) or open a past
recording. Shows the pitch contour with note segmentation, pitch accuracy
and stability numbers, vibrato, onsets, dynamics, tone indicators, a quality
verdict with reasons, and one main correction. Two recordings can be
compared (DTW-aligned contours). Recordings can be renamed, played and
deleted.

### Coach
Skill map by stage with levels and lock reasons; a lesson view with warm-up,
objective, demo, recording, measured feedback, one correction, retry and
self-rating; range check (lowest/highest comfortable notes); session plan
(reviews due first, then new material, demanding work skipped while
resting); always-visible "I feel discomfort" button; exercise library.

### Song Lab
For songs the user legally owns: import a backing track (and optionally a
reference vocal), enter or import lyrics (plain text or LRC), add kana
readings, see romaji, set line timings by tapping along, group lines into
phrases, loop a phrase at reduced speed without changing pitch, record
multiple attempts, compare an attempt's pitch contour with the reference,
keep notes per song.

### Recording Studio
Projects with tracks (arm, monitor, input channel, mute/solo, gain, pan,
colour), timeline with clips (move, trim, split, duplicate, delete, fades,
gain), recording with count-in and metronome, take groups with comping and
flattening, loop region, markers, tempo, undo/redo with labels, save,
autosave and revision history with recovery, missing-audio relink, WAV
import (converted to the project rate), mixer with per-track and master
effect chains (built-in effects with presets, VST3 plugins), export to WAV
(range, tail, rate, bit depth, mono/stereo, dither) with a verification
report.

### Plugins
Scan standard and user VST3 folders (each plugin scanned in isolation),
list usable effects and failures with reasons, enable/disable, forget,
explain limitations. In the Studio, plugin slots show status (running,
crashed, missing, disabled), generic parameter sliders, Reload after a
crash, and relink for missing plugins.

### Progress
Skill history charts (score per attempt, level), attempt list, recordings
library, practice notes.

### Settings
Microphone on/off, input/output device, sample rate, buffer size, input
channel, latency offset, reference volume, kana/romaji display, analysis
hop and lightweight mode, session length, autosave interval, data folders,
backup library, delete practice history. (The voice range is set by the
Coach's range check; plugin folders are managed on the Plugins screen.)
