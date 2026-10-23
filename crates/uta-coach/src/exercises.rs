//! Exercise library and plan generation.
//!
//! An [`ExercisePlan`] is fully determined by (exercise, level, voice range):
//! the notes the singer is asked for, the guide melody, timings and the
//! thresholds used to grade the attempt. Targets are timed relative to the
//! start of the recording, which the engine starts on the same audio frame
//! as the guide sequence.

use serde::{Deserialize, Serialize};
use uta_analysis::compare::TargetNote;
use uta_dsp::tone::note_name;

use crate::skills::MAX_LEVEL;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExerciseKind {
    PitchMatch,
    Scale,
    Sustain,
    Siren,
    OctaveSlide,
    Swell,
    Rhythm,
    Onset,
    Vowels,
    PhraseLength,
    Articulation,
    ColorContrast,
    PhraseShape,
    Arpeggio,
    Vibrato,
    Texture,
    MoraTiming,
    MelodyVowels,
    Warmup,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExerciseDef {
    pub id: &'static str,
    pub skill: &'static str,
    pub kind: ExerciseKind,
    pub title: &'static str,
    pub objective: &'static str,
    pub steps: &'static [&'static str],
    /// Suggested warm-up exercise id.
    pub warmup: Option<&'static str>,
    pub safety: Option<&'static str>,
}

const GENERAL_SAFETY: &str = "Sing at a comfortable volume. Stop and rest if you feel pain, throat discomfort, or hoarseness.";

pub static EXERCISES: &[ExerciseDef] = &[
    ExerciseDef { id: "warmup_siren", skill: "smooth_transitions", kind: ExerciseKind::Warmup, title: "Warm-up: gentle siren on “oo”", objective: "Wake the voice up gently with a soft, easy glide.", steps: &["Sing a soft “oo” (う) like a quiet siren, low to medium-high and back.", "Keep it light — this is not a test.", "Repeat two or three times."], warmup: None, safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "warmup_hum", skill: "pitch_accuracy", kind: ExerciseKind::Warmup, title: "Warm-up: humming five-note scale", objective: "Hum along with an easy five-note scale.", steps: &["Listen to the scale.", "Hum it with lips closed and jaw relaxed.", "Keep the hum quiet and buzzy at the lips."], warmup: None, safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "pitch_match", skill: "pitch_matching", kind: ExerciseKind::PitchMatch, title: "Match the note", objective: "Hear one note and sing it back on “ah” (あ).", steps: &["Listen to the reference note.", "After the count-in, sing the same note on “ah” and hold it until the bar ends.", "Aim for the middle of the note, not a slide up into it."], warmup: Some("warmup_hum"), safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "scale_5", skill: "pitch_accuracy", kind: ExerciseKind::Scale, title: "Five-note scale", objective: "Sing a short scale with each note centred.", steps: &["Listen to the scale.", "Sing it on “la” (ら) with the clicks.", "Change notes cleanly on each click."], warmup: Some("warmup_hum"), safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "onset_clean", skill: "gentle_onset", kind: ExerciseKind::Onset, title: "Clean, gentle starts", objective: "Start each note right on pitch without a hard click or a scoop.", steps: &["Breathe in easily before each note.", "Start “ah” as if you were starting a gentle sigh — not a cough, not a breathy fade-in.", "Sing each note when its click sounds."], warmup: Some("warmup_siren"), safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "vowels_aiueo", skill: "vowel_clarity", kind: ExerciseKind::Vowels, title: "あ・い・う・え・お on one note", objective: "Change vowels while the pitch stays perfectly still.", steps: &["Listen to the note.", "Sing あ い う え お, one vowel per click, all on that note.", "Only the mouth shape changes — the pitch and loudness stay the same."], warmup: Some("warmup_hum"), safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "rhythm_ta", skill: "rhythm", kind: ExerciseKind::Rhythm, title: "“Ta” with the click", objective: "Start each syllable exactly on the beat.", steps: &["Listen to the click.", "Say or sing “ta” (た) on each beat after the count-in.", "Keep each “ta” short and crisp."], warmup: None, safety: None },
    ExerciseDef { id: "sustain_hold", skill: "sustain_stability", kind: ExerciseKind::Sustain, title: "Hold it steady", objective: "Hold one note without drifting flat/sharp or wobbling.", steps: &["Listen to the note.", "Sing it on “ah” and hold it steadily until the tone ends.", "Keep the breath flowing evenly; don't push."], warmup: Some("warmup_siren"), safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "phrase_breath", skill: "phrase_length", kind: ExerciseKind::PhraseLength, title: "Long, easy breath", objective: "Sustain an even tone for the target time on one breath.", steps: &["Take a relaxed, low breath.", "Sing an easy “oo” at a comfortable, medium volume.", "Stop before you feel any strain — even if the timer is still running."], warmup: Some("warmup_siren"), safety: Some("Never squeeze out the last bit of air. Stopping early is fine and is still recorded honestly.") },
    ExerciseDef { id: "siren_glide", skill: "smooth_transitions", kind: ExerciseKind::Siren, title: "Smooth siren", objective: "Glide up and down without breaks or jumps.", steps: &["Listen to the guide glide.", "Glide on “ng” or “oo”, slow and connected, following the guide.", "If the voice flips, make the glide lighter, not louder."], warmup: Some("warmup_siren"), safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "octave_slide", skill: "register_coordination", kind: ExerciseKind::OctaveSlide, title: "Even octave slide", objective: "Slide through an octave keeping the sound and volume even.", steps: &["Listen to the guide.", "Slide up and back down on “oo” with an even volume.", "Let the top get lighter rather than shouting it."], warmup: Some("warmup_siren"), safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "articulate_mamemimo", skill: "articulation", kind: ExerciseKind::Articulation, title: "ま・め・み・も・む evenly", objective: "Repeat syllables with even timing and even loudness.", steps: &["Listen to the click.", "Sing ま め み も む … one syllable per click on the given note.", "Keep every syllable the same length and loudness."], warmup: Some("warmup_hum"), safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "swell_messa", skill: "dynamic_contrast", kind: ExerciseKind::Swell, title: "Soft → full → soft", objective: "Swell the volume up and back down while the pitch stays put.", steps: &["Start soft.", "Grow to a comfortable fuller sound in the middle — never shouting.", "Return to soft by the end. Keep the pitch steady throughout."], warmup: Some("warmup_siren"), safety: Some("More volume is not better. Grade depends on control, not loudness. Stop at any discomfort.") },
    ExerciseDef { id: "color_dark_bright", skill: "resonance_choices", kind: ExerciseKind::ColorContrast, title: "Dark colour, then bright colour", objective: "Sing one note with a darker colour, then a brighter colour.", steps: &["First half: a rounder, darker “oh” (お) — like a yawn-ish space.", "Second half: a brighter, more forward “eh” (え).", "Keep the same pitch and roughly the same volume."], warmup: Some("warmup_hum"), safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "phrase_arc", skill: "dramatic_phrasing", kind: ExerciseKind::PhraseShape, title: "Build and release", objective: "Shape a phrase: build to the top note, then release.", steps: &["Listen to the phrase.", "Sing it on “la”, growing towards the highest note…", "…then relax the volume as the phrase comes down."], warmup: Some("warmup_hum"), safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "rhythm_eighths", skill: "rhythmic_precision", kind: ExerciseKind::Rhythm, title: "Faster rhythm", objective: "Keep syllables on time at faster subdivisions.", steps: &["Listen to the click and the pattern.", "Sing “ta” on every subdivision after the count-in.", "Stay relaxed — tension makes you rush."], warmup: None, safety: None },
    ExerciseDef { id: "arpeggio_leaps", skill: "fast_transitions", kind: ExerciseKind::Arpeggio, title: "Arpeggio leaps", objective: "Land each leap in tune at tempo.", steps: &["Listen to the arpeggio.", "Sing it on “ya” (や) with the guide.", "Think of each top note as light and easy."], warmup: Some("warmup_siren"), safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "vibrato_onoff", skill: "ornamentation", kind: ExerciseKind::Vibrato, title: "Straight tone, then vibrato", objective: "Choose when vibrato happens: first straight, then with vibrato.", steps: &["Hold the note perfectly straight for the first half.", "When the second click sounds, let a relaxed, even vibrato appear.", "Don't force the vibrato with the throat or jaw."], warmup: Some("warmup_siren"), safety: Some(GENERAL_SAFETY) },
    ExerciseDef { id: "texture_intro", skill: "stylistic_texture", kind: ExerciseKind::Texture, title: "Guided texture (introductory)", objective: "Explore a gentle edge/texture at low volume while staying comfortable.", steps: &["First sing the note cleanly at a comfortable level (reference).", "Then add a small amount of edge — like a quiet, playful growl at the start of the note — without getting louder.", "If anything scratches, burns or tickles: stop and rest. Clean singing is always fine."], warmup: Some("warmup_siren"), safety: Some("Texture work must never hurt. Keep it quiet and short. This app cannot see inside your throat; if you feel pain, hoarseness or discomfort, stop and consult a qualified voice teacher or medical professional.") },
    ExerciseDef { id: "mora_kotoba", skill: "mora_timing", kind: ExerciseKind::MoraTiming, title: "Mora timing: た・っ・た・た", objective: "Give every mora one beat — including the silent beat of っ.", steps: &["Listen to the click.", "Sing the pattern: one mora per click. っ is a silent beat (stop the sound).", "Keep each mora the same length."], warmup: None, safety: None },
    ExerciseDef { id: "melody_vowels", skill: "japanese_vowels", kind: ExerciseKind::MelodyVowels, title: "Vowels on a moving melody", objective: "Keep pure vowels while the melody moves.", steps: &["Listen to the melody.", "Sing あ い う え お, one per note.", "Keep vowels pure: う with relaxed lips, not a rounded “oo”."], warmup: Some("warmup_hum"), safety: Some(GENERAL_SAFETY) },
];

pub fn exercise(id: &str) -> Option<&'static ExerciseDef> {
    EXERCISES.iter().find(|e| e.id == id)
}

pub fn exercises_for_skill(skill: &str) -> impl Iterator<Item = &'static ExerciseDef> + '_ {
    EXERCISES.iter().filter(move |e| e.skill == skill && e.kind != ExerciseKind::Warmup)
}

/// The singer's comfortable range (MIDI). Exercises stay inside it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceRange {
    pub low: f32,
    pub high: f32,
}

impl Default for VoiceRange {
    /// A conservative default for an untrained adult male voice (A2–D4).
    /// Replaced by the user's own measured comfortable range.
    fn default() -> Self {
        Self { low: 45.0, high: 62.0 }
    }
}

impl VoiceRange {
    pub fn center(&self) -> f32 {
        ((self.low + self.high) / 2.0).round()
    }
    pub fn span(&self) -> f32 {
        self.high - self.low
    }
    pub fn clamp(&self, m: f32) -> f32 {
        m.clamp(self.low, self.high)
    }
    pub fn sanitized(self) -> Self {
        let low = self.low.clamp(28.0, 84.0);
        let high = self.high.clamp(low + 5.0, 96.0);
        Self { low, high }
    }
}

/// A guide note played by the engine (`audible` = played during the attempt).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuideNote {
    pub midi: f32,
    pub start: f32,
    pub duration: f32,
}

/// Grading thresholds (only the ones relevant to the exercise kind are used).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Criteria {
    pub pitch_tolerance_cents: f32,
    pub min_in_tune_ratio: f32,
    pub max_drift_cents: f32,
    pub max_wobble_cents: f32,
    pub timing_tolerance_ms: f32,
    pub min_contrast_db: f32,
    pub max_contrast_db: f32,
    pub min_duration_s: f32,
    pub max_level_spread_db: f32,
    pub min_brightness_change: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExercisePlan {
    pub exercise_id: String,
    pub skill_id: String,
    pub kind: ExerciseKind,
    pub level: u32,
    pub title: String,
    pub objective: String,
    pub steps: Vec<String>,
    pub safety: Option<String>,
    /// Demonstration played before the attempt.
    pub demo: Vec<GuideNote>,
    /// Played during the attempt (support at low levels; may be empty).
    pub guide: Vec<GuideNote>,
    pub guide_volume: f32,
    /// Click tempo during the attempt.
    pub click_bpm: Option<f32>,
    /// Graded notes, relative to recording start.
    pub targets: Vec<TargetNote>,
    /// Syllable / lyric for each target (for display).
    pub syllables: Vec<String>,
    /// Expected syllable onsets for rhythm-type exercises (seconds).
    pub beats: Vec<f32>,
    pub record_seconds: f32,
    pub octave_tolerant: bool,
    pub criteria: Criteria,
    /// Human-readable note names used (for the UI).
    pub note_labels: Vec<String>,
}

fn level_clamp(level: u32) -> u32 {
    level.clamp(1, MAX_LEVEL)
}

/// Deterministic pick from a list using a seed (attempt counter), so plans
/// vary between attempts but are reproducible in tests.
fn pick<T: Copy>(items: &[T], seed: u64) -> T {
    items[(seed as usize) % items.len()]
}

struct Builder {
    plan: ExercisePlan,
}

impl Builder {
    fn new(def: &ExerciseDef, level: u32) -> Self {
        Self {
            plan: ExercisePlan {
                exercise_id: def.id.into(),
                skill_id: def.skill.into(),
                kind: def.kind,
                level,
                title: def.title.into(),
                objective: def.objective.into(),
                steps: def.steps.iter().map(|s| s.to_string()).collect(),
                safety: def.safety.map(String::from),
                demo: vec![],
                guide: vec![],
                guide_volume: 0.15,
                click_bpm: None,
                targets: vec![],
                syllables: vec![],
                beats: vec![],
                record_seconds: 4.0,
                octave_tolerant: true,
                criteria: Criteria::default(),
                note_labels: vec![],
            },
        }
    }

    /// Notes sung one per beat after a one-bar count-in at `bpm`.
    fn melody(&mut self, midis: &[f32], beats_each: f32, bpm: f32, guide_during: bool) {
        let beat = 60.0 / bpm;
        let lead = 4.0 * beat;
        let mut t = lead;
        for &m in midis {
            let dur = beats_each * beat;
            self.plan.targets.push(TargetNote { midi: m, start: t, duration: dur * 0.95 });
            self.plan.demo.push(GuideNote { midi: m, start: t - lead, duration: dur * 0.9 });
            if guide_during {
                self.plan.guide.push(GuideNote { midi: m, start: t, duration: dur * 0.9 });
            }
            t += dur;
        }
        self.plan.click_bpm = Some(bpm);
        self.plan.record_seconds = t + 0.8;
        self.labels();
    }

    /// A single held note after a count-in.
    fn held(&mut self, midi: f32, seconds: f32, bpm: f32) {
        let lead = 4.0 * 60.0 / bpm;
        self.plan.targets.push(TargetNote { midi, start: lead, duration: seconds });
        self.plan.demo.push(GuideNote { midi, start: 0.0, duration: 1.5 });
        self.plan.click_bpm = Some(bpm);
        self.plan.record_seconds = lead + seconds + 0.8;
        self.labels();
    }

    fn rhythm(&mut self, bpm: f32, subdivision: u32, count: u32, pattern: Option<&[bool]>) {
        let beat = 60.0 / bpm;
        let lead = 4.0 * beat;
        let step = beat / subdivision as f32;
        for i in 0..count {
            let on = pattern.map(|p| p[i as usize % p.len()]).unwrap_or(true);
            if on {
                self.plan.beats.push(lead + i as f32 * step);
            }
        }
        self.plan.click_bpm = Some(bpm);
        self.plan.record_seconds = lead + count as f32 * step + 0.8;
    }

    fn labels(&mut self) {
        let mut v: Vec<String> = self.plan.targets.iter().map(|t| note_name(t.midi)).collect();
        v.dedup();
        self.plan.note_labels = v;
    }
}

/// Build the plan for an exercise at a level within the singer's range.
/// `seed` varies note choice between attempts (pass the attempt count).
pub fn build_plan(def: &ExerciseDef, level: u32, range: VoiceRange, seed: u64) -> ExercisePlan {
    let level = level_clamp(level);
    let r = range.sanitized();
    let c = r.center();
    let mut b = Builder::new(def, level);
    let l = level as usize - 1;
    match def.kind {
        ExerciseKind::Warmup => {
            if def.id == "warmup_hum" {
                let m: Vec<f32> = [-4.0, -2.0, 0.0, 1.0, 3.0, 1.0, 0.0, -2.0, -4.0].iter().map(|o| r.clamp(c + o)).collect();
                b.melody(&m, 1.0, 80.0, true);
            } else {
                b.plan.demo = glide_demo((c - 5.0).max(r.low), (c + 5.0).min(r.high), 3.0);
                b.plan.record_seconds = 8.0;
            }
            b.plan.criteria.pitch_tolerance_cents = 100.0;
        }
        ExerciseKind::PitchMatch => {
            let spread = [3.0, 5.0, 7.0, r.span() / 2.0, r.span() / 2.0][l];
            let offsets: Vec<f32> = (-(spread as i32)..=(spread as i32)).map(|x| x as f32).collect();
            let m = r.clamp(c + pick(&offsets, seed.wrapping_mul(7919)));
            if level >= 5 {
                let m2 = r.clamp(m + pick(&[-5.0, -3.0, 2.0, 4.0], seed));
                let m3 = r.clamp(m + pick(&[-2.0, 3.0, 5.0], seed / 3));
                b.melody(&[m, m2, m3], 2.0, 60.0, false);
            } else {
                b.held(m, [2.0, 2.0, 2.5, 2.5, 2.5][l], 80.0);
            }
            b.plan.criteria.pitch_tolerance_cents = [50.0, 35.0, 25.0, 20.0, 15.0][l];
            b.plan.criteria.min_in_tune_ratio = 1.0;
            b.plan.criteria.max_wobble_cents = [40.0, 30.0, 25.0, 20.0, 15.0][l];
        }
        ExerciseKind::Scale => {
            let (pattern, bpm, guide): (&[f32], f32, bool) = match level {
                1 => (&[0.0, 2.0, 4.0, 2.0, 0.0], 60.0, true),
                2 => (&[0.0, 2.0, 4.0, 5.0, 7.0, 5.0, 4.0, 2.0, 0.0], 70.0, true),
                3 => (&[0.0, 2.0, 4.0, 5.0, 7.0, 5.0, 4.0, 2.0, 0.0], 80.0, false),
                4 => (&[0.0, 2.0, 4.0, 5.0, 7.0, 9.0, 11.0, 12.0, 7.0, 4.0, 0.0], 90.0, false),
                _ => (&[0.0, 2.0, 3.0, 5.0, 7.0, 8.0, 7.0, 5.0, 3.0, 2.0, 0.0], 100.0, false),
            };
            // fall back to the simplest pattern if the range is too narrow
            let pattern: &[f32] = if pattern.iter().fold(0.0f32, |m, v| m.max(*v)) > r.span() { &[0.0, 2.0, 4.0, 2.0, 0.0] } else { pattern };
            let top = pattern.iter().fold(0.0f32, |m, v| m.max(*v));
            let root = (c - (top / 2.0).round()).min(r.high - top).max(r.low);
            let midis: Vec<f32> = pattern.iter().map(|p| root + p).collect();
            b.melody(&midis, 1.0, bpm, guide);
            b.plan.criteria.pitch_tolerance_cents = if level == 1 { 50.0 } else { [50.0, 40.0, 30.0, 25.0, 25.0][l] };
            b.plan.criteria.min_in_tune_ratio = [0.6, 0.7, 0.8, 0.8, 0.85][l];
        }
        ExerciseKind::Sustain => {
            let m = r.clamp(c + pick(&[-2.0, 0.0, 2.0, -1.0, 1.0], seed));
            b.held(m, [3.0, 5.0, 7.0, 9.0, 12.0][l], 80.0);
            b.plan.criteria.pitch_tolerance_cents = [40.0, 30.0, 25.0, 20.0, 15.0][l];
            b.plan.criteria.max_drift_cents = [40.0, 30.0, 25.0, 20.0, 15.0][l];
            b.plan.criteria.max_wobble_cents = [25.0, 20.0, 15.0, 12.0, 10.0][l];
        }
        ExerciseKind::PhraseLength => {
            let m = r.clamp(c - 2.0);
            let secs = [6.0, 9.0, 12.0, 15.0, 18.0][l];
            b.held(m, secs, 80.0);
            b.plan.criteria.min_duration_s = secs * 0.9;
            b.plan.criteria.max_level_spread_db = [6.0, 5.0, 4.0, 3.5, 3.0][l];
            b.plan.criteria.pitch_tolerance_cents = 50.0;
            b.plan.criteria.max_drift_cents = 50.0;
        }
        ExerciseKind::Siren | ExerciseKind::OctaveSlide => {
            let span = if def.kind == ExerciseKind::OctaveSlide { 12.0f32.min(r.span()) } else { [5.0, 7.0, 9.0, 11.0, r.span()][l].min(r.span()) };
            let lo = (c - span / 2.0).max(r.low);
            let hi = (lo + span).min(r.high);
            let secs = [4.0, 3.5, 3.0, 2.5, 2.0][l];
            b.plan.demo = glide_demo(lo, hi, secs);
            let lead = 3.0 * 60.0 / 80.0;
            b.plan.targets = vec![
                TargetNote { midi: lo, start: lead, duration: 0.4 },
                TargetNote { midi: hi, start: lead + secs / 2.0 - 0.2, duration: 0.4 },
                TargetNote { midi: lo, start: lead + secs - 0.4, duration: 0.4 },
            ];
            b.plan.click_bpm = Some(80.0);
            b.plan.record_seconds = lead + secs + 1.0;
            b.labels();
            b.plan.criteria.pitch_tolerance_cents = 100.0;
            b.plan.criteria.min_in_tune_ratio = 0.8; // fraction of span covered
            b.plan.criteria.max_level_spread_db = [12.0, 10.0, 9.0, 8.0, 6.0][l];
        }
        ExerciseKind::Swell => {
            let m = r.clamp(c);
            b.held(m, [4.0, 5.0, 6.0, 7.0, 8.0][l], 80.0);
            b.plan.criteria.min_contrast_db = [4.0, 6.0, 7.0, 8.0, 9.0][l];
            b.plan.criteria.max_contrast_db = 20.0;
            b.plan.criteria.max_drift_cents = [50.0, 40.0, 30.0, 25.0, 20.0][l];
            b.plan.criteria.pitch_tolerance_cents = 40.0;
        }
        ExerciseKind::Rhythm => {
            if def.id == "rhythm_eighths" {
                let syncopated: &[bool] = &[true, false, true, true, false, true, true, false];
                match level {
                    1 => b.rhythm(80.0, 2, 16, None),
                    2 => b.rhythm(95.0, 2, 16, None),
                    3 => b.rhythm(80.0, 2, 16, Some(syncopated)),
                    4 => b.rhythm(100.0, 2, 16, Some(syncopated)),
                    _ => b.rhythm(80.0, 4, 16, None),
                }
                b.plan.criteria.timing_tolerance_ms = [60.0, 50.0, 45.0, 40.0, 35.0][l];
            } else {
                let bpm = [70.0, 80.0, 90.0, 100.0, 110.0][l];
                b.rhythm(bpm, 1, 8, None);
                b.plan.criteria.timing_tolerance_ms = [80.0, 65.0, 55.0, 45.0, 35.0][l];
            }
            b.plan.criteria.min_in_tune_ratio = 0.85; // fraction of beats matched
            b.plan.syllables = vec!["た".into(); b.plan.beats.len()];
        }
        ExerciseKind::Onset => {
            let m = r.clamp(c);
            b.melody(&[m, m, m, m], 2.0, 80.0, false);
            b.plan.syllables = vec!["あ".into(); 4];
            b.plan.criteria.pitch_tolerance_cents = [60.0, 45.0, 35.0, 30.0, 25.0][l];
            b.plan.criteria.max_drift_cents = [80.0, 60.0, 45.0, 35.0, 30.0][l]; // allowed onset scoop
            b.plan.criteria.min_in_tune_ratio = 0.75;
        }
        ExerciseKind::Vowels => {
            let m = r.clamp(c - 1.0);
            let beats = [2.0, 1.5, 1.0, 1.0, 0.5][l];
            let bpm = [80.0, 80.0, 80.0, 100.0, 80.0][l];
            b.melody(&[m; 5], beats, bpm, level <= 2);
            b.plan.syllables = ["あ", "い", "う", "え", "お"].iter().map(|s| s.to_string()).collect();
            b.plan.criteria.pitch_tolerance_cents = [50.0, 40.0, 30.0, 25.0, 25.0][l];
            b.plan.criteria.max_wobble_cents = [30.0, 25.0, 20.0, 18.0, 15.0][l];
            b.plan.criteria.min_in_tune_ratio = 0.8;
        }
        ExerciseKind::Articulation => {
            let m = r.clamp(c);
            let bpm = [70.0, 80.0, 90.0, 100.0, 120.0][l];
            b.rhythm(bpm, if level >= 4 { 2 } else { 1 }, 10, None);
            b.plan.targets = vec![TargetNote { midi: m, start: b.plan.beats[0], duration: b.plan.beats.last().copied().unwrap_or(0.0) - b.plan.beats[0] + 0.3 }];
            b.plan.demo = vec![GuideNote { midi: m, start: 0.0, duration: 1.5 }];
            b.labels();
            let syl = ["ま", "め", "み", "も", "む"];
            b.plan.syllables = (0..b.plan.beats.len()).map(|i| syl[i % 5].to_string()).collect();
            b.plan.criteria.timing_tolerance_ms = [80.0, 65.0, 55.0, 45.0, 40.0][l];
            b.plan.criteria.max_level_spread_db = [6.0, 5.0, 4.0, 3.5, 3.0][l];
            b.plan.criteria.min_in_tune_ratio = 0.8;
        }
        ExerciseKind::ColorContrast => {
            let m = r.clamp(c - 1.0);
            b.held(m, 6.0, 80.0);
            b.plan.criteria.min_brightness_change = [0.10, 0.15, 0.2, 0.25, 0.3][l];
            b.plan.criteria.pitch_tolerance_cents = 40.0;
            b.plan.criteria.max_contrast_db = 6.0; // loudness should stay similar
        }
        ExerciseKind::PhraseShape => {
            let root = r.clamp(c - 3.0);
            let p = [0.0, 2.0, 4.0, 5.0, 7.0, 5.0, 4.0, 2.0, 0.0];
            let midis: Vec<f32> = p.iter().map(|x| (root + x).min(r.high)).collect();
            b.melody(&midis, 1.0, [60.0, 70.0, 75.0, 80.0, 90.0][l], level <= 2);
            b.plan.criteria.pitch_tolerance_cents = 40.0;
            b.plan.criteria.min_in_tune_ratio = 0.7;
            b.plan.criteria.min_contrast_db = [3.0, 4.0, 5.0, 6.0, 6.0][l];
        }
        ExerciseKind::Arpeggio => {
            let root = (c - 6.0).max(r.low);
            let top = (root + 12.0).min(r.high);
            let midis = [root, root + 4.0, root + 7.0, top, root + 7.0, root + 4.0, root].map(|m| r.clamp(m));
            b.melody(&midis, 1.0, [70.0, 85.0, 100.0, 115.0, 130.0][l], level <= 2);
            b.plan.criteria.pitch_tolerance_cents = [50.0, 40.0, 35.0, 30.0, 25.0][l];
            b.plan.criteria.min_in_tune_ratio = [0.6, 0.7, 0.75, 0.8, 0.85][l];
        }
        ExerciseKind::Vibrato => {
            let m = r.clamp(c);
            b.held(m, 5.0, 80.0);
            b.plan.criteria.max_wobble_cents = 15.0; // straight half
            b.plan.criteria.pitch_tolerance_cents = 40.0;
        }
        ExerciseKind::Texture => {
            let m = r.clamp(c - 2.0);
            b.melody(&[m, m], 4.0, 80.0, false);
            b.plan.syllables = vec!["clean".into(), "texture".into()];
            b.plan.criteria.max_contrast_db = 3.0; // texture take must not be louder than +3 dB
            b.plan.criteria.pitch_tolerance_cents = 60.0;
        }
        ExerciseKind::MoraTiming => {
            let patterns: [(&[&str], &[bool]); 3] = [
                (&["た", "っ", "た", "た"], &[true, false, true, true]),
                (&["か", "っ", "ぱ", "た"], &[true, false, true, true]),
                (&["き", "っ", "て", "た"], &[true, false, true, true]),
            ];
            let (syl, pat) = patterns[(seed as usize) % patterns.len()];
            let bpm = [70.0, 80.0, 95.0, 110.0, 125.0][l];
            b.rhythm(bpm, 1, 8, Some(pat));
            b.plan.syllables = (0..8).map(|i| syl[i % 4].to_string()).collect();
            b.plan.criteria.timing_tolerance_ms = [80.0, 65.0, 55.0, 45.0, 40.0][l];
            b.plan.criteria.min_in_tune_ratio = 0.85;
        }
        ExerciseKind::MelodyVowels => {
            let root = r.clamp(c - 2.0);
            let p = [[0.0, 2.0, 4.0, 2.0, 0.0], [0.0, 4.0, 2.0, 5.0, 0.0], [0.0, 5.0, 4.0, 7.0, 2.0]];
            let pp = p[(level as usize - 1).min(2)];
            let midis: Vec<f32> = pp.iter().map(|x| (root + x).min(r.high)).collect();
            b.melody(&midis, [2.0, 1.5, 1.0, 1.0, 1.0][l], 80.0, level <= 2);
            b.plan.syllables = ["あ", "い", "う", "え", "お"].iter().map(|s| s.to_string()).collect();
            b.plan.criteria.pitch_tolerance_cents = [50.0, 40.0, 30.0, 25.0, 25.0][l];
            b.plan.criteria.min_in_tune_ratio = 0.75;
        }
    }
    b.plan
}

fn glide_demo(lo: f32, hi: f32, seconds: f32) -> Vec<GuideNote> {
    // stepwise approximation of a glide for the tone generator (it glides
    // between consecutive notes)
    let steps = 12;
    let mut v = Vec::new();
    for i in 0..=steps {
        let x = i as f32 / steps as f32;
        let tri = if x < 0.5 { x * 2.0 } else { 2.0 - x * 2.0 };
        v.push(GuideNote { midi: lo + (hi - lo) * tri, start: x * seconds, duration: seconds / steps as f32 + 0.01 });
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_exercise_builds_at_every_level_inside_range() {
        let ranges = [VoiceRange::default(), VoiceRange { low: 40.0, high: 70.0 }, VoiceRange { low: 50.0, high: 56.0 }];
        for def in EXERCISES {
            assert!(crate::skills::skill(def.skill).is_some(), "{} has unknown skill", def.id);
            for range in ranges {
                let rs = range.sanitized();
                for level in 1..=MAX_LEVEL {
                    for seed in 0..4 {
                        let p = build_plan(def, level, range, seed);
                        assert!(p.record_seconds > 1.0, "{}", def.id);
                        assert!(p.record_seconds < 40.0, "{}", def.id);
                        for t in &p.targets {
                            assert!(t.midi >= rs.low - 0.01 && t.midi <= rs.high + 0.01, "{} L{level}: {} outside {:?}", def.id, t.midi, rs);
                            assert!(t.start >= 0.0 && t.end() <= p.record_seconds, "{} timing", def.id);
                        }
                        for g in p.demo.iter().chain(&p.guide) {
                            assert!(g.midi >= rs.low - 0.01 && g.midi <= rs.high + 0.01, "{} guide outside range", def.id);
                        }
                        for w in p.beats.windows(2) {
                            assert!(w[1] > w[0]);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn difficulty_increases_with_level() {
        let def = exercise("sustain_hold").unwrap();
        let p1 = build_plan(def, 1, VoiceRange::default(), 0);
        let p5 = build_plan(def, 5, VoiceRange::default(), 0);
        assert!(p5.targets[0].duration > p1.targets[0].duration);
        assert!(p5.criteria.max_drift_cents < p1.criteria.max_drift_cents);
        let r1 = build_plan(exercise("rhythm_ta").unwrap(), 1, VoiceRange::default(), 0);
        let r5 = build_plan(exercise("rhythm_ta").unwrap(), 5, VoiceRange::default(), 0);
        assert!(r5.click_bpm.unwrap() > r1.click_bpm.unwrap());
        assert!(r5.criteria.timing_tolerance_ms < r1.criteria.timing_tolerance_ms);
    }
}
