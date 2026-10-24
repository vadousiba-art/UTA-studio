# Training method

The coach is a deterministic, rule-based trainer ("Layer A", `uta-coach`).
It needs no AI model, no network and no GPU, and every decision it makes can
be explained in one sentence. Optional local AI layers (a local language
model for explanations, audio models) are **not** part of this version; see
[known-limitations.md](known-limitations.md).

It teaches transferable technique — pitch, breath, onset, vowels, rhythm,
dynamics, registers, phrasing — that serves expressive J-pop/J-rock styles
(including Ado-inspired dynamics), without imitating any singer's voice.

> Practice tool only — not a substitute for a qualified voice teacher or a
> medical professional.

## Curriculum

Six stages, 22 skills, each with levels 1–5. A skill unlocks when its
prerequisites reach the stated level. Lesson counts never unlock anything.

| Stage | Skills (prerequisites) |
| --- | --- |
| 1 Foundation | Pitch matching · Scale accuracy (pitch matching 1) · Gentle onset · Vowel clarity あいうえお (pitch matching 1) · Rhythm |
| 2 Control | Sustained-note stability (pitch matching 2) · Breath phrasing (sustain 1) · Smooth transitions (scale accuracy 2) · Register coordination (transitions 1) · Consistent articulation (rhythm 2) |
| 3 Expressive character | Soft-to-strong contrast (sustain 2) · Resonance colour (vowels 2, sustain 2) · Dramatic phrasing (contrast 1, scale accuracy 2) · Rhythmic precision (rhythm 3) |
| 4 Advanced style | Fast register transitions (registers 2, scale accuracy 3) · Ornamentation and vibrato (sustain 3) · Guided stylistic texture (sustain 3, contrast 2, registers 2, **and no discomfort reports in the last 14 days**) |
| 5 Japanese song application | Mora timing (rhythm 2, vowels 1) · Japanese vowels in melody (vowels 2, scale accuracy 2) · Lyric phrase alignment (mora timing 1, scale accuracy 2) — uses Song Lab |
| 6 Performance & production | Full-song practice (lyric phrase 2) · Doubles, harmonies and mixing (full song 1) — uses the Studio |

21 exercises implement these skills (pitch match, five-note scale, clean
onsets, あいうえお on one note, "ta" with the click, sustain, long breath,
siren, octave slide, まめみもむ articulation, messa di voce swell, dark/bright
colour, build-and-release phrase, faster rhythm, arpeggio leaps, straight
tone vs vibrato, guided texture, mora timing た・っ・た・た, vowels on a
moving melody, plus two warm-ups). Each exercise is generated for the
singer's **comfortable range** (default A2–D4 until the range check is
done) and level: higher levels mean longer holds, wider intervals, faster
tempos and tighter tolerances — never louder or higher notes for their own
sake.

## Lesson loop

1. **Warm-up** suggestion (gentle siren or humming scale).
2. **Objective** — one sentence on what to listen for.
3. **Demonstration** — the guide notes/clicks are played (reference volume
   is adjustable and starts low).
4. **Attempt** — count-in, guide, recording; live pitch display.
5. **Measured feedback** — numbers from the analysis (e.g. median cents off,
   drift, wobble, onset timing, dynamic range), plus a quality check.
6. **One correction** — the single most important thing to change, phrased
   as an action.
7. **Retry**, then an optional **self-rating** of difficulty (Easy … Too hard).
8. **Adaptation** using the rules below; **review** is scheduled.

The **discomfort button** is visible during every exercise. Pressing it
stops the exercise immediately.

## Evaluation

* Each exercise defines criteria (pitch tolerance in cents, minimum voiced
  time, stability, timing windows, etc.) scaled by level.
* A recording that is too quiet, clipped, noisy, mostly unvoiced, or too
  short is graded **Inconclusive** — the app says why and asks for a clean
  recording instead of guessing.
* Octave errors are folded (singing the target an octave lower is fine for
  pitch exercises) — the singer is never pushed into a register.
* Outcomes: Success, Partial, Needs work, Inconclusive. Loudness and high
  notes are never rewarded: swells are graded on control and pitch
  steadiness, breath exercises allow stopping early.

## Adaptation rules (`progression.rs`)

| Rule | Condition |
| --- | --- |
| Level up | The last 3 conclusive attempts at the current level were all Success. |
| Simplify (level down) | 2 of the last 3 conclusive attempts at the current level were Needs work. |
| Keep | Anything else. |
| Request a clean recording | 2 Inconclusive attempts in a row. |
| Stop | Discomfort reported. |

## Spaced review

Leitner boxes with intervals **1, 2, 4, 8, 16, 32 days**. Success moves an
exercise up a box; Needs work moves it back to box 0; Partial keeps the
box; Inconclusive doesn't count. Due reviews are recommended before new
material.

## Vocal health rules

* Discomfort (pain, throat discomfort, hoarseness) → the exercise stops, the
  app recommends rest, and **demanding skills** (soft-to-strong contrast,
  texture, fast register transitions, register coordination, dramatic
  phrasing, breath phrasing) are not recommended for **2 days**.
* Reports on **2 or more different days within 14 days** → the app
  recommends seeing a qualified voice teacher or medical professional
  (doctor / speech-language pathologist).
* Texture work (edge, gentle grit) is introductory, low-volume, unlocked
  only after the prerequisites **and** a clean 14-day health window, and
  stops at any discomfort.
* Session length is configurable; recommended sessions are short (10–20
  minutes).

## Song Lab connection

Japanese song stage exercises use Song Lab: phrase loops at reduced speed
(pitch unchanged), kana/romaji lyrics, multiple attempts with pitch-contour
comparison against a reference the user provides, and notes.
