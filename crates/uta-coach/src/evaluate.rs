//! Deterministic grading of an attempt and generation of feedback.
//!
//! Every statement in the feedback is derived from a measurement computed
//! here. Nothing is invented: when the recording does not support a
//! judgement (noise, clipping, too little singing), the outcome is
//! [`Outcome::Inconclusive`] and the feedback says why.
//!
//! Never interpreted: loudness, brightness or height as "better" singing;
//! anything about vocal-fold physiology or health.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uta_analysis::compare::TargetNote;
use uta_analysis::notes::analyze_sustain;
use uta_analysis::onsets::{detect_onsets, match_onsets};
use uta_analysis::phrase::{analyze_phrase, AnalysisOptions, PhraseAnalysis};
use uta_analysis::pitch::PitchFrame;
use uta_analysis::quality::{QualityIssue, QualityLevel};
use uta_analysis::spectral::analyze_spectrum;
use uta_analysis::stats::{mean, median, std_dev};
use uta_analysis::vibrato::detect_vibrato;

use crate::exercises::{exercise, ExerciseKind, ExercisePlan};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Success,
    Partial,
    NeedsWork,
    /// The recording cannot support a judgement; not counted for progress.
    Inconclusive,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Success => "success",
            Outcome::Partial => "partial",
            Outcome::NeedsWork => "needsWork",
            Outcome::Inconclusive => "inconclusive",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "success" => Outcome::Success,
            "partial" => Outcome::Partial,
            "needsWork" => Outcome::NeedsWork,
            "inconclusive" => Outcome::Inconclusive,
            _ => return None,
        })
    }
    pub fn is_conclusive(self) -> bool {
        self != Outcome::Inconclusive
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub exercise_id: String,
    pub title: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feedback {
    pub headline: String,
    pub went_well: Vec<String>,
    pub primary_opportunity: Option<String>,
    pub suggestion: Option<Suggestion>,
    pub retry_goal: Option<String>,
    pub uncertainty: Option<String>,
    pub safety_reminder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub key: String,
    pub label: String,
    pub value: String,
    pub pass: bool,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evaluation {
    pub outcome: Outcome,
    /// 0..1, only meaningful for conclusive outcomes.
    pub score: f32,
    /// 0..1 confidence in the measurement.
    pub confidence: f32,
    pub checks: Vec<CheckResult>,
    pub metrics: BTreeMap<String, f32>,
    pub feedback: Feedback,
}

/// One graded criterion.
struct Check {
    key: &'static str,
    label: &'static str,
    value: String,
    pass: bool,
    score: f32,
    good: String,
    fix: String,
    goal: String,
    drill: &'static str,
}

/// Linear score: 1 at `good`, 0 at `bad` (works for either direction).
fn lin(v: f32, good: f32, bad: f32) -> f32 {
    if (bad - good).abs() < 1e-9 {
        return if v == good { 1.0 } else { 0.0 };
    }
    (1.0 - (v - good) / (bad - good)).clamp(0.0, 1.0)
}

fn cents_word(c: f32) -> String {
    if c.abs() < 3.0 {
        "right on pitch".into()
    } else if c > 0.0 {
        format!("{:.0} cents sharp", c)
    } else {
        format!("{:.0} cents flat", -c)
    }
}

fn shifted(targets: &[TargetNote], latency: f32) -> Vec<TargetNote> {
    targets
        .iter()
        .map(|t| TargetNote { start: t.start + latency, ..*t })
        .collect()
}

fn frames_in(frames: &[PitchFrame], start: f32, end: f32) -> Vec<PitchFrame> {
    frames.iter().filter(|f| f.time >= start && f.time <= end).copied().collect()
}

fn voiced_ratio(frames: &[PitchFrame]) -> f32 {
    if frames.is_empty() {
        0.0
    } else {
        frames.iter().filter(|f| f.is_voiced()).count() as f32 / frames.len() as f32
    }
}

fn quality_message(a: &PhraseAnalysis, relevant: impl Fn(QualityIssue) -> bool) -> Option<String> {
    let msgs: Vec<&str> = a.quality.issues.iter().copied().filter(|i| relevant(*i)).map(|i| i.message()).collect();
    if msgs.is_empty() {
        None
    } else {
        Some(msgs.join(" "))
    }
}

fn inconclusive(reason: String, a: Option<&PhraseAnalysis>, plan: &ExercisePlan) -> Evaluation {
    Evaluation {
        outcome: Outcome::Inconclusive,
        score: 0.0,
        confidence: a.and_then(|a| a.quality.mean_confidence).unwrap_or(0.0),
        checks: vec![],
        metrics: BTreeMap::new(),
        feedback: Feedback {
            headline: "Let's try that recording again".into(),
            went_well: vec![],
            primary_opportunity: None,
            suggestion: None,
            retry_goal: Some("Record again so the take can be measured reliably.".into()),
            uncertainty: Some(reason),
            safety_reminder: plan.safety.clone(),
        },
    }
}

fn finish(plan: &ExercisePlan, checks: Vec<Check>, mut metrics: BTreeMap<String, f32>, confidence: f32, uncertainty: Option<String>, extra_note: Option<String>) -> Evaluation {
    let all_pass = checks.iter().all(|c| c.pass);
    let score = if checks.is_empty() {
        0.0
    } else {
        checks.iter().map(|c| c.score).sum::<f32>() / checks.len() as f32
    };
    let outcome = if all_pass {
        Outcome::Success
    } else if score >= 0.6 {
        Outcome::Partial
    } else {
        Outcome::NeedsWork
    };
    let mut went_well: Vec<String> = checks.iter().filter(|c| c.pass).map(|c| c.good.clone()).take(2).collect();
    if let Some(n) = extra_note {
        went_well.push(n);
    }
    let worst = checks.iter().filter(|c| !c.pass).min_by(|a, b| a.score.total_cmp(&b.score));
    let suggestion = worst.and_then(|w| {
        let def = exercise(w.drill)?;
        Some(Suggestion {
            exercise_id: def.id.into(),
            title: def.title.into(),
            reason: format!("It targets exactly this: {}", def.objective.to_lowercase()),
        })
    });
    let headline = match outcome {
        Outcome::Success => "Nice — that met every goal for this level.".to_string(),
        Outcome::Partial => "Good attempt — one thing to tidy up.".to_string(),
        Outcome::NeedsWork => "Let's focus on one thing and try again.".to_string(),
        Outcome::Inconclusive => unreachable!(),
    };
    metrics.insert("score".into(), score);
    Evaluation {
        outcome,
        score,
        confidence,
        checks: checks
            .iter()
            .map(|c| CheckResult { key: c.key.into(), label: c.label.into(), value: c.value.clone(), pass: c.pass, score: c.score })
            .collect(),
        metrics,
        feedback: Feedback {
            headline,
            went_well: if went_well.is_empty() { vec!["You completed the attempt and it could be measured.".into()] } else { went_well },
            primary_opportunity: worst.map(|w| w.fix.clone()),
            suggestion,
            retry_goal: Some(worst.map(|w| w.goal.clone()).unwrap_or_else(|| "Repeat it once more to confirm it's consistent.".into())),
            uncertainty,
            safety_reminder: plan.safety.clone(),
        },
    }
}

/// Grade an attempt. `samples` is the mono recording that started on the
/// same frame as the plan's guide; `latency` is the round-trip
/// output→input latency estimate in seconds (shifts expected times).
pub fn evaluate(plan: &ExercisePlan, samples: &[f32], sample_rate: u32, latency: f32) -> Evaluation {
    let latency = latency.clamp(0.0, 0.5);
    let targets = shifted(&plan.targets, latency);
    let opts = AnalysisOptions { targets: targets.clone(), octave_tolerant: plan.octave_tolerant, hop_seconds: 0.01 };
    let a = analyze_phrase(samples, sample_rate, &opts);
    let conf = a.quality.mean_confidence.unwrap_or(0.0);
    let is_rhythm = matches!(plan.kind, ExerciseKind::Rhythm | ExerciseKind::MoraTiming);

    // recording-quality gate
    if a.quality.level == QualityLevel::Poor {
        let relevant = |i: QualityIssue| {
            if is_rhythm {
                matches!(i, QualityIssue::TooQuiet | QualityIssue::Clipping | QualityIssue::Noisy | QualityIssue::TooShort)
            } else {
                true
            }
        };
        if let Some(msg) = quality_message(&a, relevant) {
            return inconclusive(msg, Some(&a), plan);
        }
    }
    let uncertainty = if a.quality.level == QualityLevel::Fair {
        quality_message(&a, |i| i != QualityIssue::BackgroundUnknown).map(|m| format!("Measurements are less certain: {m}"))
    } else {
        None
    };
    let mut metrics = BTreeMap::new();
    metrics.insert("meanConfidence".into(), conf);
    if let Some(snr) = a.quality.snr_db {
        metrics.insert("snrDb".into(), snr);
    }

    match plan.kind {
        ExerciseKind::Warmup => {
            if a.pitch.voiced_seconds < 1.0 {
                return inconclusive("Very little singing was detected. Warm-ups aren't graded — just sing along gently.".into(), Some(&a), plan);
            }
            metrics.insert("voicedSeconds".into(), a.pitch.voiced_seconds);
            let checks = vec![Check {
                key: "done",
                label: "Warm-up",
                value: format!("{:.1} s of easy singing", a.pitch.voiced_seconds),
                pass: true,
                score: 1.0,
                good: "Warm-up done — your voice is ready.".into(),
                fix: String::new(),
                goal: String::new(),
                drill: "warmup_siren",
            }];
            finish(plan, checks, metrics, conf, uncertainty, None)
        }
        ExerciseKind::PitchMatch => eval_pitch_notes(plan, &a, metrics, conf, uncertainty, "pitch_match"),
        ExerciseKind::Scale | ExerciseKind::Arpeggio | ExerciseKind::MelodyVowels => {
            let drill = if plan.kind == ExerciseKind::Arpeggio { "scale_5" } else { "pitch_match" };
            eval_pitch_notes(plan, &a, metrics, conf, uncertainty, drill)
        }
        ExerciseKind::Onset => eval_onset(plan, &a, metrics, conf, uncertainty),
        ExerciseKind::Sustain | ExerciseKind::Vowels => eval_sustain(plan, &targets, &a, metrics, conf, uncertainty),
        ExerciseKind::PhraseLength => eval_phrase_length(plan, &targets, &a, metrics, conf, uncertainty),
        ExerciseKind::Siren | ExerciseKind::OctaveSlide => eval_glide(plan, &targets, &a, metrics, conf, uncertainty),
        ExerciseKind::Swell => eval_swell(plan, &targets, &a, metrics, conf, uncertainty),
        ExerciseKind::Rhythm | ExerciseKind::MoraTiming | ExerciseKind::Articulation => {
            eval_rhythm(plan, samples, sample_rate, latency, &targets, &a, metrics, conf, uncertainty)
        }
        ExerciseKind::ColorContrast => eval_color(plan, samples, sample_rate, &targets, &a, metrics, conf, uncertainty),
        ExerciseKind::PhraseShape => eval_phrase_shape(plan, &targets, &a, metrics, conf, uncertainty),
        ExerciseKind::Vibrato => eval_vibrato(plan, &targets, &a, metrics, conf, uncertainty),
        ExerciseKind::Texture => eval_texture(plan, samples, sample_rate, &targets, &a, metrics, conf, uncertainty),
    }
}

fn eval_pitch_notes(plan: &ExercisePlan, a: &PhraseAnalysis, mut metrics: BTreeMap<String, f32>, conf: f32, unc: Option<String>, drill: &'static str) -> Evaluation {
    let Some(cmp) = &a.targets else {
        return inconclusive("No target notes to compare with.".into(), Some(a), plan);
    };
    let total = cmp.notes.len().max(1);
    let measurable = cmp.measurable_notes;
    if measurable == 0 || (measurable as f32) < total as f32 * 0.5 {
        return inconclusive(
            format!("Only {measurable} of {total} notes had enough steady singing to measure. Start singing right after the count-in and hold each note for its whole beat."),
            Some(a),
            plan,
        );
    }
    let tol = plan.criteria.pitch_tolerance_cents;
    let errs: Vec<f32> = cmp.notes.iter().filter_map(|n| n.error_cents).collect();
    let in_tune = errs.iter().filter(|e| e.abs() <= tol).count() as f32 / errs.len() as f32;
    let mean_abs = cmp.mean_abs_error_cents.unwrap_or(999.0);
    let mean_signed = mean(&errs).unwrap_or(0.0);
    metrics.insert("meanAbsErrorCents".into(), mean_abs);
    metrics.insert("meanErrorCents".into(), mean_signed);
    metrics.insert("inTuneRatio".into(), in_tune);
    metrics.insert("measurableNotes".into(), measurable as f32);
    let mut checks = Vec::new();
    let single = total == 1;
    let tend = if mean_signed > 8.0 { "sharp" } else if mean_signed < -8.0 { "flat" } else { "" };
    checks.push(Check {
        key: "pitch",
        label: "Pitch accuracy",
        value: if single { cents_word(errs[0]) } else { format!("{:.0}% of notes within ±{tol:.0}¢ (avg {:.0}¢ off)", in_tune * 100.0, mean_abs) },
        pass: in_tune >= plan.criteria.min_in_tune_ratio.max(0.01) && (!single || errs[0].abs() <= tol),
        score: if single { lin(errs[0].abs(), tol * 0.5, tol * 3.0) } else { lin(in_tune, 1.0, plan.criteria.min_in_tune_ratio - 0.4) },
        good: if single { format!("You landed the note {}.", cents_word(errs[0])) } else { format!("{:.0}% of your notes were in tune.", in_tune * 100.0) },
        fix: if single {
            format!("Your held note was {}. {}", cents_word(errs[0]), if errs[0] < 0.0 { "Think slightly higher and keep the breath moving." } else { "Relax and let the note settle a touch lower." })
        } else if tend.is_empty() {
            format!("Some notes missed the centre (average {:.0} cents off). Listen to the guide once more before singing.", mean_abs)
        } else {
            format!("Your notes tended to be {tend} (average {:.0} cents). {}", mean_signed.abs(), if tend == "flat" { "Aim each note a little higher, especially going down the scale." } else { "Relax upward notes so they don't overshoot." })
        },
        goal: if single { format!("Land within ±{tol:.0} cents.") } else { format!("Get at least {:.0}% of notes within ±{tol:.0} cents.", plan.criteria.min_in_tune_ratio * 100.0) },
        drill,
    });
    let wobbles: Vec<f32> = cmp.notes.iter().filter_map(|n| n.wobble_cents).collect();
    if plan.criteria.max_wobble_cents > 0.0 {
        if let Some(w) = median(&wobbles) {
            metrics.insert("wobbleCents".into(), w);
            checks.push(Check {
                key: "steadiness",
                label: "Steadiness",
                value: format!("{w:.0}¢ wobble"),
                pass: w <= plan.criteria.max_wobble_cents,
                score: lin(w, plan.criteria.max_wobble_cents * 0.5, plan.criteria.max_wobble_cents * 2.5),
                good: "The note stayed steady once you found it.".into(),
                fix: format!("The pitch wavered by about {w:.0} cents while holding. Use a slightly softer, even breath flow."),
                goal: format!("Keep the wobble under {:.0} cents.", plan.criteria.max_wobble_cents),
                drill: "sustain_hold",
            });
        }
    }
    let oct = cmp.notes.iter().find(|n| n.octave_shift != 0).map(|n| n.octave_shift);
    let note = oct.map(|o| format!("You sang {} octave{} {} the reference — octave matches count, so that's fine.", o.abs(), if o.abs() > 1 { "s" } else { "" }, if o < 0 { "below" } else { "above" }));
    finish(plan, checks, metrics, conf, unc, note)
}

fn eval_onset(plan: &ExercisePlan, a: &PhraseAnalysis, mut metrics: BTreeMap<String, f32>, conf: f32, unc: Option<String>) -> Evaluation {
    let Some(cmp) = &a.targets else {
        return inconclusive("No target notes.".into(), Some(a), plan);
    };
    if cmp.measurable_notes < 2 {
        return inconclusive("Fewer than two notes could be measured — sing each “ah” on its click.".into(), Some(a), plan);
    }
    // scoop: first 80 ms of each note's voiced frames vs the note median
    let mut scoops = Vec::new();
    for n in &cmp.notes {
        let Some(sung) = n.sung_midi else { continue };
        let fr = frames_in(&a.frames, n.target.start - 0.1, n.target.end());
        let voiced: Vec<&PitchFrame> = fr.iter().filter(|f| f.is_voiced()).collect();
        if voiced.len() < 10 {
            continue;
        }
        let start_t = voiced[0].time;
        let early: Vec<f32> = voiced.iter().filter(|f| f.time < start_t + 0.08).filter_map(|f| f.midi()).collect();
        let fold = 12.0 * n.octave_shift as f32;
        if let Some(e) = median(&early) {
            scoops.push(((e - fold) - sung) * 100.0);
        }
    }
    let scoop = median(&scoops).unwrap_or(0.0);
    metrics.insert("onsetScoopCents".into(), scoop);
    let errs: Vec<f32> = cmp.notes.iter().filter_map(|n| n.error_cents).collect();
    let tol = plan.criteria.pitch_tolerance_cents;
    let in_tune = errs.iter().filter(|e| e.abs() <= tol).count() as f32 / errs.len().max(1) as f32;
    metrics.insert("inTuneRatio".into(), in_tune);
    let max_scoop = plan.criteria.max_drift_cents;
    let checks = vec![
        Check {
            key: "scoop",
            label: "Clean start",
            value: if scoop.abs() < 10.0 { "starts on pitch".into() } else { format!("starts {:.0}¢ {}", scoop.abs(), if scoop < 0.0 { "below" } else { "above" }) },
            pass: scoop.abs() <= max_scoop,
            score: lin(scoop.abs(), max_scoop * 0.4, max_scoop * 2.5),
            good: "Your notes started close to the target pitch — no big scoop.".into(),
            fix: format!("Notes started about {:.0} cents {} the target and slid into place. Imagine the note before you start it, then begin softly right on it.", scoop.abs(), if scoop < 0.0 { "below" } else { "above" }),
            goal: format!("Start within {max_scoop:.0} cents of the target."),
            drill: "pitch_match",
        },
        Check {
            key: "pitch",
            label: "Held pitch",
            value: format!("{:.0}% in tune", in_tune * 100.0),
            pass: in_tune >= plan.criteria.min_in_tune_ratio,
            score: lin(in_tune, 1.0, 0.3),
            good: "The held part of each note was in tune.".into(),
            fix: "Some held notes were off target; listen to the reference again before starting.".into(),
            goal: format!("Keep {:.0}% of notes within ±{tol:.0} cents.", plan.criteria.min_in_tune_ratio * 100.0),
            drill: "pitch_match",
        },
    ];
    finish(plan, checks, metrics, conf, unc, None)
}

fn eval_sustain(plan: &ExercisePlan, targets: &[TargetNote], a: &PhraseAnalysis, mut metrics: BTreeMap<String, f32>, conf: f32, unc: Option<String>) -> Evaluation {
    let start = targets.first().map(|t| t.start).unwrap_or(0.0);
    let end = targets.last().map(|t| t.end()).unwrap_or(a.duration);
    let target_midi = targets.first().map(|t| t.midi).unwrap_or(60.0);
    let fr = frames_in(&a.frames, start, end);
    let cover = voiced_ratio(&fr);
    metrics.insert("coverage".into(), cover);
    if cover < 0.3 {
        return inconclusive(
            format!("You were singing for only {:.0}% of the hold. Start after the count-in and keep going until the end.", cover * 100.0),
            Some(a),
            plan,
        );
    }
    let mut fr_folded = fr.clone();
    // octave folding: keep the singer's octave choice
    let med_midi = median(&fr.iter().filter_map(|f| f.midi()).collect::<Vec<_>>()).unwrap_or(target_midi);
    let shift = ((med_midi - target_midi) / 12.0).round();
    if plan.octave_tolerant && shift != 0.0 {
        for f in &mut fr_folded {
            f.hz = f.hz.map(|h| h / 2f32.powf(shift));
        }
    }
    let Some(s) = analyze_sustain(&fr_folded, Some(target_midi)) else {
        return inconclusive("The held note was too short or broken up to measure steadiness.".into(), Some(a), plan);
    };
    let err = s.target_error_cents.unwrap_or(0.0);
    metrics.insert("errorCents".into(), err);
    metrics.insert("driftCents".into(), s.drift_cents);
    metrics.insert("wobbleCents".into(), s.wobble_cents);
    let cr = &plan.criteria;
    let mut checks = vec![Check {
        key: "pitch",
        label: "Average pitch",
        value: cents_word(err),
        pass: err.abs() <= cr.pitch_tolerance_cents,
        score: lin(err.abs(), cr.pitch_tolerance_cents * 0.5, cr.pitch_tolerance_cents * 3.0),
        good: format!("Your average pitch was {}.", cents_word(err)),
        fix: format!("On average the note was {}. Listen to the reference again and match its centre.", cents_word(err)),
        goal: format!("Keep the average within ±{:.0} cents.", cr.pitch_tolerance_cents),
        drill: "pitch_match",
    }];
    if plan.kind == ExerciseKind::Sustain {
        checks.push(Check {
            key: "drift",
            label: "Drift",
            value: format!("{:+.0}¢ over the hold", s.drift_cents),
            pass: s.drift_cents.abs() <= cr.max_drift_cents,
            score: lin(s.drift_cents.abs(), cr.max_drift_cents * 0.5, cr.max_drift_cents * 3.0),
            good: "You didn't drift — the end of the note matched the start.".into(),
            fix: format!(
                "The pitch drifted {} by {:.0} cents across the hold. {}",
                if s.drift_cents < 0.0 { "down (flat)" } else { "up (sharp)" },
                s.drift_cents.abs(),
                if s.drift_cents < 0.0 { "This usually happens as breath energy fades — keep the airflow steady to the end." } else { "Avoid pushing harder as the note goes on." }
            ),
            goal: format!("Drift less than {:.0} cents from start to end.", cr.max_drift_cents),
            drill: "sustain_hold",
        });
        checks.push(Check {
            key: "coverage",
            label: "Held for",
            value: format!("{:.1} s of {:.1} s", s.duration, end - start),
            pass: cover >= 0.8,
            score: lin(cover, 1.0, 0.4),
            good: "You held the note for the full time.".into(),
            fix: format!("The note was held for {:.1} of {:.1} seconds. A shorter hold that stays even is better than pushing on.", s.duration, end - start),
            goal: "Hold for the full time, comfortably.".into(),
            drill: "phrase_breath",
        });
    }
    checks.push(Check {
        key: "wobble",
        label: "Steadiness",
        value: format!("{:.0}¢ wobble", s.wobble_cents),
        pass: s.wobble_cents <= cr.max_wobble_cents.max(1.0),
        score: lin(s.wobble_cents, cr.max_wobble_cents * 0.5, cr.max_wobble_cents * 3.0),
        good: "The note was steady, with little wobble.".into(),
        fix: format!(
            "The pitch wavered by about {:.0} cents{}. Try a slightly lighter sound with a calm, even breath.",
            s.wobble_cents,
            if plan.kind == ExerciseKind::Vowels { " when the vowels changed" } else { "" }
        ),
        goal: format!("Keep wobble under {:.0} cents.", cr.max_wobble_cents),
        drill: "sustain_hold",
    });
    if let Some(v) = s.vibrato {
        metrics.insert("vibratoRateHz".into(), v.rate_hz);
        metrics.insert("vibratoExtentCents".into(), v.extent_cents);
    }
    let note = if plan.kind == ExerciseKind::Vowels {
        Some("Vowel identity itself isn't checked automatically — listen back and compare your あ い う え お with the guide.".to_string())
    } else {
        s.vibrato.map(|v| format!("Some natural vibrato was detected ({:.1} Hz, ±{:.0}¢); it isn't counted as wobble.", v.rate_hz, v.extent_cents))
    };
    finish(plan, checks, metrics, conf, unc, note)
}

fn eval_phrase_length(plan: &ExercisePlan, targets: &[TargetNote], a: &PhraseAnalysis, mut metrics: BTreeMap<String, f32>, conf: f32, unc: Option<String>) -> Evaluation {
    let start = targets.first().map(|t| t.start - 0.3).unwrap_or(0.0);
    let longest = a.notes.iter().filter(|n| n.end > start).map(|n| n.duration()).fold(0.0f32, f32::max);
    // also accept one continuous voiced run with small pitch changes
    let fr = frames_in(&a.frames, start, a.duration);
    let mut run = 0.0f32;
    let mut best_run = 0.0f32;
    for f in &fr {
        if f.is_voiced() {
            run += a.hop_seconds;
            best_run = best_run.max(run);
        } else {
            run = 0.0;
        }
    }
    let held = longest.max(best_run);
    metrics.insert("heldSeconds".into(), held);
    if held < 1.0 {
        return inconclusive("No sustained tone was detected. Start singing after the count-in.".into(), Some(a), plan);
    }
    let levels: Vec<f32> = fr.iter().filter(|f| f.is_voiced()).map(|f| f.rms_db).collect();
    let trimmed = if levels.len() > 60 { &levels[30..levels.len() - 30] } else { &levels[..] };
    let spread = std_dev(trimmed).unwrap_or(0.0);
    metrics.insert("levelSpreadDb".into(), spread);
    let cr = &plan.criteria;
    let checks = vec![
        Check {
            key: "duration",
            label: "Length on one breath",
            value: format!("{held:.1} s"),
            pass: held >= cr.min_duration_s,
            score: lin(held, cr.min_duration_s, cr.min_duration_s * 0.4),
            good: format!("You sustained an easy tone for {held:.1} seconds."),
            fix: format!("You sustained {held:.1} s (goal {:.0} s). Length grows with relaxed, low breathing over many sessions — never squeeze out the last air.", cr.min_duration_s),
            goal: format!("Sustain {:.0} s while it still feels easy.", cr.min_duration_s),
            drill: "phrase_breath",
        },
        Check {
            key: "evenness",
            label: "Evenness",
            value: format!("±{spread:.1} dB"),
            pass: spread <= cr.max_level_spread_db,
            score: lin(spread, cr.max_level_spread_db * 0.5, cr.max_level_spread_db * 2.5),
            good: "The tone stayed even in volume.".into(),
            fix: format!("The volume varied by about {spread:.1} dB. Aim for a medium, even level rather than a big start that fades."),
            goal: format!("Keep the level within ±{:.0} dB.", cr.max_level_spread_db),
            drill: "phrase_breath",
        },
    ];
    finish(plan, checks, metrics, conf, unc, None)
}

fn eval_glide(plan: &ExercisePlan, targets: &[TargetNote], a: &PhraseAnalysis, mut metrics: BTreeMap<String, f32>, conf: f32, unc: Option<String>) -> Evaluation {
    let start = targets.first().map(|t| t.start - 0.3).unwrap_or(0.0);
    let end = targets.last().map(|t| t.end() + 0.5).unwrap_or(a.duration);
    let lo = targets.iter().map(|t| t.midi).fold(f32::MAX, f32::min);
    let hi = targets.iter().map(|t| t.midi).fold(f32::MIN, f32::max);
    let fr = frames_in(&a.frames, start, end);
    let voiced: Vec<&PitchFrame> = fr.iter().filter(|f| f.is_voiced()).collect();
    if voiced.len() < 30 {
        return inconclusive("The glide was too short or quiet to measure. Glide continuously with the guide.".into(), Some(a), plan);
    }
    let midis: Vec<f32> = voiced.iter().filter_map(|f| f.midi()).collect();
    // octave fold relative to target range
    let med = median(&midis).unwrap_or(lo);
    let shift = ((med - (lo + hi) / 2.0) / 12.0).round() * 12.0;
    let mn = midis.iter().fold(f32::MAX, |m, v| m.min(v - shift));
    let mx = midis.iter().fold(f32::MIN, |m, v| m.max(v - shift));
    let coverage = ((mx.min(hi + 1.0) - mn.max(lo - 1.0)) / (hi - lo).max(1.0)).clamp(0.0, 1.0);
    // breaks: unvoiced gaps > 60 ms between first and last voiced frame
    let first = fr.iter().position(|f| f.is_voiced()).unwrap_or(0);
    let last = fr.iter().rposition(|f| f.is_voiced()).unwrap_or(0);
    let mut breaks = 0;
    let mut gap = 0;
    for f in &fr[first..=last] {
        if f.is_voiced() {
            if gap as f32 * a.hop_seconds > 0.06 {
                breaks += 1;
            }
            gap = 0;
        } else {
            gap += 1;
        }
    }
    // jumps: > 1.2 semitone between consecutive voiced frames
    let jumps = voiced
        .windows(2)
        .filter(|w| (w[1].time - w[0].time) < 0.03)
        .filter(|w| (w[1].midi().unwrap_or(0.0) - w[0].midi().unwrap_or(0.0)).abs() > 1.2)
        .count();
    let levels: Vec<f32> = voiced.iter().map(|f| f.rms_db).collect();
    let spread = std_dev(&levels).unwrap_or(0.0);
    metrics.insert("rangeCoverage".into(), coverage);
    metrics.insert("breaks".into(), breaks as f32);
    metrics.insert("jumps".into(), jumps as f32);
    metrics.insert("levelSpreadDb".into(), spread);
    let allowed_breaks = if plan.level <= 1 { 1 } else { 0 };
    let mut checks = vec![
        Check {
            key: "connected",
            label: "Connected glide",
            value: format!("{} break{}, {} jump{}", breaks, if breaks == 1 { "" } else { "s" }, jumps, if jumps == 1 { "" } else { "s" }),
            pass: breaks <= allowed_breaks && jumps == 0,
            score: lin((breaks + jumps) as f32, 0.0, 4.0),
            good: "The glide stayed connected with no breaks or flips.".into(),
            fix: format!("The sound broke or jumped {} time{} during the glide. Make the glide lighter and slower where it breaks — don't push through.", breaks + jumps, if breaks + jumps == 1 { "" } else { "s" }),
            goal: "Glide with no breaks or sudden jumps.".into(),
            drill: "siren_glide",
        },
        Check {
            key: "coverage",
            label: "Range covered",
            value: format!("{:.0}% of the glide", coverage * 100.0),
            pass: coverage >= plan.criteria.min_in_tune_ratio,
            score: lin(coverage, 1.0, 0.3),
            good: "You covered the whole glide range.".into(),
            fix: format!("You covered {:.0}% of the range. Follow the guide all the way, staying light at the top.", coverage * 100.0),
            goal: format!("Cover at least {:.0}% of the range.", plan.criteria.min_in_tune_ratio * 100.0),
            drill: "siren_glide",
        },
    ];
    if plan.kind == ExerciseKind::OctaveSlide {
        checks.push(Check {
            key: "evenness",
            label: "Even volume",
            value: format!("±{spread:.1} dB"),
            pass: spread <= plan.criteria.max_level_spread_db,
            score: lin(spread, plan.criteria.max_level_spread_db * 0.5, plan.criteria.max_level_spread_db * 2.0),
            good: "The volume stayed even through the range.".into(),
            fix: format!("Volume changed by about {spread:.1} dB across the slide. Let the top get lighter instead of louder."),
            goal: format!("Keep the level within ±{:.0} dB.", plan.criteria.max_level_spread_db),
            drill: "octave_slide",
        });
    }
    finish(plan, checks, metrics, conf, unc, None)
}

fn eval_swell(plan: &ExercisePlan, targets: &[TargetNote], a: &PhraseAnalysis, mut metrics: BTreeMap<String, f32>, conf: f32, unc: Option<String>) -> Evaluation {
    let t = targets[0];
    let fr = frames_in(&a.frames, t.start, t.end());
    let Some((s0, peak, s1)) = uta_analysis::dynamics::swell_profile(&fr) else {
        return inconclusive("The swell was too short or broken up to measure. Sustain one note for the whole time.".into(), Some(a), plan);
    };
    let contrast = peak - s0.max(s1);
    let symmetry = (s0 - s1).abs();
    metrics.insert("contrastDb".into(), contrast);
    metrics.insert("startEndDifferenceDb".into(), symmetry);
    let sustain = analyze_sustain(&fr, Some(t.midi));
    let wobble = sustain.as_ref().map(|s| s.wobble_cents.max(s.drift_cents.abs())).unwrap_or(0.0);
    metrics.insert("pitchMovementCents".into(), wobble);
    let cr = &plan.criteria;
    let checks = vec![
        Check {
            key: "contrast",
            label: "Soft-to-full contrast",
            value: format!("{contrast:.1} dB"),
            pass: contrast >= cr.min_contrast_db,
            score: lin(contrast, cr.min_contrast_db, 0.0),
            good: format!("You made a clear {contrast:.0} dB swell."),
            fix: format!("The swell was {contrast:.1} dB. Start softer rather than finishing louder — the soft ends make the contrast."),
            goal: format!("Reach about {:.0} dB of contrast, keeping the loud part comfortable.", cr.min_contrast_db),
            drill: "swell_messa",
        },
        Check {
            key: "return",
            label: "Return to soft",
            value: format!("{symmetry:.1} dB between start and end"),
            pass: symmetry <= 6.0,
            score: lin(symmetry, 2.0, 12.0),
            good: "You came back down to soft at the end.".into(),
            fix: format!("The ending was {symmetry:.0} dB different from the start. Save some control to bring the volume back down."),
            goal: "End within 6 dB of where you started.".into(),
            drill: "swell_messa",
        },
        Check {
            key: "pitch",
            label: "Pitch while swelling",
            value: format!("{wobble:.0}¢ movement"),
            pass: wobble <= cr.max_drift_cents,
            score: lin(wobble, cr.max_drift_cents * 0.5, cr.max_drift_cents * 2.5),
            good: "Your pitch stayed put while the volume changed.".into(),
            fix: format!("The pitch moved about {wobble:.0} cents as the volume changed. Getting louder often pulls sharp — keep the note centred."),
            goal: format!("Keep pitch movement under {:.0} cents.", cr.max_drift_cents),
            drill: "sustain_hold",
        },
    ];
    let note = (contrast > cr.max_contrast_db).then(|| "That was a very large swell. More volume isn't the goal — control is. Keep the loud point comfortable.".to_string());
    finish(plan, checks, metrics, conf, unc, note)
}

#[allow(clippy::too_many_arguments)]
fn eval_rhythm(plan: &ExercisePlan, samples: &[f32], sr: u32, latency: f32, targets: &[TargetNote], a: &PhraseAnalysis, mut metrics: BTreeMap<String, f32>, conf: f32, unc: Option<String>) -> Evaluation {
    let expected: Vec<f32> = plan.beats.iter().map(|b| b + latency).collect();
    if expected.is_empty() {
        return inconclusive("This exercise has no beats.".into(), Some(a), plan);
    }
    let step = if expected.len() > 1 { expected[1] - expected[0] } else { 0.5 };
    let onsets = detect_onsets(samples, sr, 10.0, (step * 0.5).min(0.12));
    let window = (step * 0.45).min(0.25);
    let matched = match_onsets(&onsets, &expected, window);
    let errs: Vec<f32> = matched.iter().flatten().copied().collect();
    let hit_ratio = errs.len() as f32 / expected.len() as f32;
    if hit_ratio < 0.3 {
        return inconclusive(
            format!("Only {} of {} syllables were detected. Make each syllable short and clear, close to the microphone.", errs.len(), expected.len()),
            Some(a),
            plan,
        );
    }
    let mean_abs = mean(&errs.iter().map(|e| e.abs()).collect::<Vec<_>>()).unwrap_or(0.0);
    let signed = mean(&errs).unwrap_or(0.0);
    let spread = std_dev(&errs).unwrap_or(0.0);
    metrics.insert("hitRatio".into(), hit_ratio);
    metrics.insert("meanAbsOffsetMs".into(), mean_abs);
    metrics.insert("meanOffsetMs".into(), signed);
    metrics.insert("spreadMs".into(), spread);
    let tol = plan.criteria.timing_tolerance_ms;
    let mut checks = vec![
        Check {
            key: "timing",
            label: "Timing",
            value: format!("±{mean_abs:.0} ms average"),
            pass: mean_abs <= tol,
            score: lin(mean_abs, tol * 0.5, tol * 2.5),
            good: format!("Your syllables landed on the beat (±{mean_abs:.0} ms on average)."),
            fix: if signed < -tol * 0.5 {
                format!("You were rushing: syllables came about {:.0} ms early. Let the click arrive, then speak.", -signed)
            } else if signed > tol * 0.5 {
                format!("You were dragging: syllables came about {signed:.0} ms late. Prepare the syllable just before the click.")
            } else {
                format!("Timing was uneven (±{mean_abs:.0} ms). Tap your foot with the click to lock in.")
            },
            goal: format!("Average within ±{tol:.0} ms of the beat."),
            drill: "rhythm_ta",
        },
        Check {
            key: "hits",
            label: "Syllables on beats",
            value: format!("{} of {}", errs.len(), expected.len()),
            pass: hit_ratio >= plan.criteria.min_in_tune_ratio,
            score: lin(hit_ratio, 1.0, 0.4),
            good: "You placed a syllable on every beat.".into(),
            fix: format!("{} beats had no clear syllable. Keep each syllable short with a clear consonant.", expected.len() - errs.len()),
            goal: format!("Hit at least {:.0}% of the beats.", plan.criteria.min_in_tune_ratio * 100.0),
            drill: "rhythm_ta",
        },
    ];
    if plan.kind == ExerciseKind::MoraTiming {
        // silent beats (っ): every expected beat skipped by the pattern
        let beat = 60.0 / plan.click_bpm.unwrap_or(80.0);
        let lead = 4.0 * beat;
        let n_beats = ((plan.record_seconds - 0.8 - lead) / beat).round() as usize;
        let silent: Vec<f32> = (0..n_beats)
            .map(|i| lead + i as f32 * beat + latency)
            .filter(|t| !expected.iter().any(|e| (e - t).abs() < 1e-3))
            .collect();
        let violations = silent
            .iter()
            .filter(|t| onsets.iter().any(|o| (o.time - **t).abs() < window * 0.8))
            .count();
        metrics.insert("silentBeatViolations".into(), violations as f32);
        checks.push(Check {
            key: "sokuon",
            label: "っ silent beat",
            value: format!("{} of {} kept silent", silent.len() - violations, silent.len()),
            pass: violations == 0,
            score: lin(violations as f32, 0.0, silent.len().max(1) as f32),
            good: "You kept the っ beats silent — that's the small-tsu timing.".into(),
            fix: "Some っ beats had sound. っ is a full beat of stopped sound: hold the consonant closed for one click.".into(),
            goal: "Keep every っ beat silent.".into(),
            drill: "mora_kotoba",
        });
    }
    if plan.kind == ExerciseKind::Articulation {
        let lvls: Vec<f32> = onsets
            .iter()
            .filter(|o| expected.iter().any(|e| (o.time - e).abs() <= window))
            .map(|o| o.level_db)
            .collect();
        let spread_db = std_dev(&lvls).unwrap_or(0.0);
        metrics.insert("syllableLevelSpreadDb".into(), spread_db);
        checks.push(Check {
            key: "evenness",
            label: "Even syllables",
            value: format!("±{spread_db:.1} dB"),
            pass: spread_db <= plan.criteria.max_level_spread_db,
            score: lin(spread_db, plan.criteria.max_level_spread_db * 0.5, plan.criteria.max_level_spread_db * 2.5),
            good: "Every syllable had about the same loudness.".into(),
            fix: format!("Syllable loudness varied by ±{spread_db:.1} dB. Keep the jaw loose and give each syllable the same energy."),
            goal: format!("Keep syllables within ±{:.0} dB.", plan.criteria.max_level_spread_db),
            drill: "articulate_mamemimo",
        });
        if let Some(t) = targets.first() {
            let fr = frames_in(&a.frames, t.start, t.end());
            let m: Vec<f32> = fr.iter().filter_map(|f| f.midi()).collect();
            if let Some(med) = median(&m) {
                let err = ((med - t.midi) - 12.0 * ((med - t.midi) / 12.0).round()) * 100.0;
                metrics.insert("pitchErrorCents".into(), err);
            }
        }
    }
    finish(plan, checks, metrics, conf, unc, None)
}

#[allow(clippy::too_many_arguments)]
fn eval_color(plan: &ExercisePlan, samples: &[f32], sr: u32, targets: &[TargetNote], a: &PhraseAnalysis, mut metrics: BTreeMap<String, f32>, conf: f32, unc: Option<String>) -> Evaluation {
    let t = targets[0];
    let mid = t.start + t.duration / 2.0;
    let (frames, _) = analyze_spectrum(samples, sr, &a.frames);
    let first: Vec<f32> = frames.iter().filter(|f| f.time > t.start + 0.3 && f.time < mid - 0.2).map(|f| f.centroid_hz).collect();
    let second: Vec<f32> = frames.iter().filter(|f| f.time > mid + 0.3 && f.time < t.end() - 0.2).map(|f| f.centroid_hz).collect();
    let (Some(c1), Some(c2)) = (median(&first), median(&second)) else {
        return inconclusive("Both halves need steady singing to compare their colour.".into(), Some(a), plan);
    };
    let change = (c2 - c1) / c1.max(1.0);
    let l1 = median(&frames_in(&a.frames, t.start, mid).iter().filter(|f| f.is_voiced()).map(|f| f.rms_db).collect::<Vec<_>>()).unwrap_or(0.0);
    let l2 = median(&frames_in(&a.frames, mid, t.end()).iter().filter(|f| f.is_voiced()).map(|f| f.rms_db).collect::<Vec<_>>()).unwrap_or(0.0);
    metrics.insert("brightnessChange".into(), change);
    metrics.insert("darkCentroidHz".into(), c1);
    metrics.insert("brightCentroidHz".into(), c2);
    metrics.insert("levelDifferenceDb".into(), l2 - l1);
    let cr = &plan.criteria;
    let checks = vec![
        Check {
            key: "colour",
            label: "Colour change",
            value: format!("{:+.0}% brightness", change * 100.0),
            pass: change >= cr.min_brightness_change,
            score: lin(change, cr.min_brightness_change, -0.05),
            good: format!("The second half was measurably brighter (+{:.0}%).", change * 100.0),
            fix: if change < 0.0 {
                "The second half came out darker than the first. Swap the colours: round “oh” first, forward “eh” second.".into()
            } else {
                format!("The colour changed by only {:.0}%. Exaggerate the difference: more space for the dark half, more forward “eh” for the bright half.", change * 100.0)
            },
            goal: format!("Make the bright half at least {:.0}% brighter.", cr.min_brightness_change * 100.0),
            drill: "color_dark_bright",
        },
        Check {
            key: "level",
            label: "Similar volume",
            value: format!("{:+.1} dB", l2 - l1),
            pass: (l2 - l1).abs() <= cr.max_contrast_db,
            score: lin((l2 - l1).abs(), 2.0, cr.max_contrast_db * 2.0),
            good: "Both halves had a similar volume — the change was colour, not loudness.".into(),
            fix: format!("The halves differed by {:.1} dB in volume. Change the colour, not the loudness.", (l2 - l1).abs()),
            goal: format!("Keep the halves within {:.0} dB.", cr.max_contrast_db),
            drill: "color_dark_bright",
        },
    ];
    let unc = unc.or(Some("Brightness is measured from the recording, so it depends on your microphone and room. Compare takes made with the same setup.".into()));
    finish(plan, checks, metrics, conf, unc, None)
}

fn eval_phrase_shape(plan: &ExercisePlan, targets: &[TargetNote], a: &PhraseAnalysis, mut metrics: BTreeMap<String, f32>, conf: f32, unc: Option<String>) -> Evaluation {
    let mut levels = Vec::new();
    for t in targets {
        let l: Vec<f32> = frames_in(&a.frames, t.start, t.end()).iter().filter(|f| f.is_voiced()).map(|f| f.rms_db).collect();
        levels.push(median(&l));
    }
    let known: Vec<(usize, f32)> = levels.iter().enumerate().filter_map(|(i, l)| l.map(|v| (i, v))).collect();
    if known.len() < targets.len() * 2 / 3 {
        return inconclusive("Too many notes were missing to judge the phrase shape.".into(), Some(a), plan);
    }
    // expected arc: rises to the highest note then falls
    let peak_idx = targets.iter().enumerate().max_by(|x, y| x.1.midi.total_cmp(&y.1.midi)).map(|(i, _)| i).unwrap_or(0);
    let shape: Vec<f32> = known
        .iter()
        .map(|(i, _)| 1.0 - (*i as f32 - peak_idx as f32).abs() / (targets.len() as f32 / 2.0))
        .collect();
    let lv: Vec<f32> = known.iter().map(|(_, v)| *v).collect();
    let corr = correlation(&shape, &lv);
    let contrast = lv.iter().fold(f32::MIN, |m, v| m.max(*v)) - lv.iter().fold(f32::MAX, |m, v| m.min(*v));
    metrics.insert("shapeCorrelation".into(), corr);
    metrics.insert("contrastDb".into(), contrast);
    let mut eval = eval_pitch_notes(plan, a, metrics, conf, unc, "scale_5");
    if eval.outcome == Outcome::Inconclusive {
        return eval;
    }
    let shape_ok = corr >= 0.5 && contrast >= plan.criteria.min_contrast_db;
    eval.checks.push(CheckResult {
        key: "shape".into(),
        label: "Build and release".into(),
        value: format!("shape match {:.2}, {:.1} dB range", corr, contrast),
        pass: shape_ok,
        score: lin(corr, 0.8, 0.0),
    });
    eval.metrics.insert("shapeCorrelation".into(), corr);
    if !shape_ok {
        eval.feedback.primary_opportunity = Some(format!(
            "The volume shape {}. Grow gradually toward the top note, then let each note after it get a little softer.",
            if contrast < plan.criteria.min_contrast_db { format!("was quite flat ({contrast:.1} dB)") } else { "didn't peak at the top note".to_string() }
        ));
        eval.feedback.retry_goal = Some(format!("Peak at the top note with about {:.0} dB of build.", plan.criteria.min_contrast_db));
        eval.outcome = if eval.outcome == Outcome::Success { Outcome::Partial } else { eval.outcome };
        eval.feedback.headline = "Good notes — now shape the phrase.".into();
    }
    eval.score = eval.checks.iter().map(|c| c.score).sum::<f32>() / eval.checks.len() as f32;
    eval
}

fn eval_vibrato(plan: &ExercisePlan, targets: &[TargetNote], a: &PhraseAnalysis, mut metrics: BTreeMap<String, f32>, conf: f32, unc: Option<String>) -> Evaluation {
    let t = targets[0];
    let mid = t.start + t.duration / 2.0;
    let cents_of = |s: f32, e: f32| -> Vec<f32> {
        frames_in(&a.frames, s, e).iter().filter_map(|f| f.midi()).map(|m| (m - t.midi) * 100.0).collect()
    };
    let straight = cents_of(t.start + 0.2, mid);
    let vib = cents_of(mid + 0.2, t.end());
    if straight.len() < 60 || vib.len() < 60 {
        return inconclusive("Both halves need about two seconds of steady singing.".into(), Some(a), plan);
    }
    let v1 = detect_vibrato(&straight, a.hop_seconds);
    let v2 = detect_vibrato(&vib, a.hop_seconds);
    metrics.insert("straightVibratoCents".into(), v1.map(|v| v.extent_cents).unwrap_or(0.0));
    if let Some(v) = v2 {
        metrics.insert("vibratoRateHz".into(), v.rate_hz);
        metrics.insert("vibratoExtentCents".into(), v.extent_cents);
    }
    let v2_ok = v2.map(|v| (4.0..=7.5).contains(&v.rate_hz) && (15.0..=90.0).contains(&v.extent_cents)).unwrap_or(false);
    let checks = vec![
        Check {
            key: "straight",
            label: "Straight first half",
            value: v1.map(|v| format!("vibrato ±{:.0}¢", v.extent_cents)).unwrap_or_else(|| "straight".into()),
            pass: v1.is_none(),
            score: if v1.is_none() { 1.0 } else { lin(v1.map(|v| v.extent_cents).unwrap_or(0.0), 15.0, 60.0) },
            good: "You held a straight tone when asked.".into(),
            fix: "Vibrato appeared in the straight half. Choosing *not* to use vibrato is part of control — keep it plain until the second click.".into(),
            goal: "Keep the first half free of vibrato.".into(),
            drill: "sustain_hold",
        },
        Check {
            key: "vibrato",
            label: "Vibrato on cue",
            value: v2.map(|v| format!("{:.1} Hz, ±{:.0}¢", v.rate_hz, v.extent_cents)).unwrap_or_else(|| "none detected".into()),
            pass: v2_ok,
            score: if v2_ok { 1.0 } else if v2.is_some() { 0.5 } else { 0.0 },
            good: "A relaxed, even vibrato appeared on cue.".into(),
            fix: match v2 {
                None => "No regular vibrato was detected in the second half. Let it come from a relaxed, steady breath rather than forcing it.".into(),
                Some(v) => format!("The vibrato measured {:.1} Hz, ±{:.0}¢. A typical relaxed vibrato is about 4.5–7 Hz and ±20–60¢.", v.rate_hz, v.extent_cents),
            },
            goal: "Let a 4.5–7 Hz vibrato of about ±20–60¢ appear in the second half.".into(),
            drill: "vibrato_onoff",
        },
    ];
    finish(plan, checks, metrics, conf, unc, None)
}

#[allow(clippy::too_many_arguments)]
fn eval_texture(plan: &ExercisePlan, samples: &[f32], sr: u32, targets: &[TargetNote], a: &PhraseAnalysis, mut metrics: BTreeMap<String, f32>, conf: f32, unc: Option<String>) -> Evaluation {
    if targets.len() < 2 {
        return inconclusive("Missing reference.".into(), Some(a), plan);
    }
    let level = |t: &TargetNote| median(&frames_in(&a.frames, t.start + 0.2, t.end()).iter().filter(|f| f.rms_db > -60.0).map(|f| f.rms_db).collect::<Vec<_>>());
    let (Some(clean), Some(tex)) = (level(&targets[0]), level(&targets[1])) else {
        return inconclusive("Both the clean note and the texture note need to be sung.".into(), Some(a), plan);
    };
    let (frames, _) = analyze_spectrum(samples, sr, &a.frames);
    let hr = |t: &TargetNote| median(&frames.iter().filter(|f| f.time >= t.start && f.time <= t.end()).map(|f| f.high_ratio_db).collect::<Vec<_>>());
    let diff = tex - clean;
    metrics.insert("levelIncreaseDb".into(), diff);
    if let (Some(h1), Some(h2)) = (hr(&targets[0]), hr(&targets[1])) {
        metrics.insert("highFrequencyChangeDb".into(), h2 - h1);
    }
    let checks = vec![Check {
        key: "level",
        label: "Stayed quiet",
        value: format!("{diff:+.1} dB vs clean note"),
        pass: diff <= plan.criteria.max_contrast_db,
        score: lin(diff, 0.0, 9.0),
        good: "You added texture without getting louder — that's the safe way to explore it.".into(),
        fix: format!("The texture note was {diff:.1} dB louder than the clean note. Texture must not come from pushing: keep it at the clean note's volume or softer."),
        goal: format!("Stay within +{:.0} dB of the clean note.", plan.criteria.max_contrast_db),
        drill: "texture_intro",
    }];
    let unc = Some(unc.unwrap_or_default() + " The app can't judge whether a texture is healthy for your voice — only how loud it was. Any discomfort means stop.");
    finish(plan, checks, metrics, conf, unc.map(|s| s.trim().to_string()), None)
}

fn correlation(x: &[f32], y: &[f32]) -> f32 {
    let n = x.len().min(y.len());
    if n < 3 {
        return 0.0;
    }
    let mx = x[..n].iter().sum::<f32>() / n as f32;
    let my = y[..n].iter().sum::<f32>() / n as f32;
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        sxy += (x[i] - mx) * (y[i] - my);
        sxx += (x[i] - mx).powi(2);
        syy += (y[i] - my).powi(2);
    }
    if sxx <= 0.0 || syy <= 0.0 {
        0.0
    } else {
        sxy / (sxx.sqrt() * syy.sqrt())
    }
}

/// Feedback for free singing in the Vocal Analyzer (no targets).
pub fn free_singing_feedback(a: &PhraseAnalysis) -> Feedback {
    let mut fb = Feedback::default();
    if a.quality.level == QualityLevel::Poor {
        fb.headline = "This take can't be measured reliably".into();
        fb.uncertainty = Some(a.quality.issues.iter().map(|i| i.message()).collect::<Vec<_>>().join(" "));
        fb.retry_goal = Some("Record again: 15–30 cm from the mic, quiet room, input peaks below the red zone.".into());
        return fb;
    }
    fb.headline = "Here's what the recording shows".into();
    if let (Some(lo), Some(hi)) = (a.pitch.low_midi, a.pitch.high_midi) {
        fb.went_well.push(format!(
            "You used a range of {} to {} ({:.0} semitones).",
            uta_dsp::tone::note_name(lo),
            uta_dsp::tone::note_name(hi),
            hi - lo
        ));
    }
    let mut opp: Option<(f32, String, String, &str)> = None;
    if let Some(s) = &a.sustain {
        if s.wobble_cents <= 12.0 && s.drift_cents.abs() <= 25.0 {
            fb.went_well.push(format!("Your longest note ({:.1} s) was steady (±{:.0}¢).", s.duration, s.wobble_cents));
        } else if s.drift_cents.abs() > 25.0 {
            opp = Some((s.drift_cents.abs() / 25.0, format!("Your longest note drifted {:.0} cents {} over {:.1} s.", s.drift_cents.abs(), if s.drift_cents < 0.0 { "flat" } else { "sharp" }, s.duration), "Hold a note with less than 25 cents of drift.".into(), "sustain_hold"));
        } else {
            opp = Some((s.wobble_cents / 12.0, format!("Your longest note wobbled about ±{:.0} cents.", s.wobble_cents), "Hold a note with less than 12 cents of wobble.".into(), "sustain_hold"));
        }
        if let Some(v) = s.vibrato {
            fb.went_well.push(format!("Natural vibrato detected: {:.1} Hz, ±{:.0}¢.", v.rate_hz, v.extent_cents));
        }
    }
    if let Some(r) = a.dynamics.dynamic_range_db {
        if r < 3.0 && opp.is_none() {
            opp = Some((1.0, format!("Your volume stayed very even ({r:.1} dB range). Expressive phrases often use more soft-to-strong contrast."), "Try one phrase with a gentle swell.".into(), "swell_messa"));
        }
    }
    if a.dynamics.clipped_samples > 0 {
        fb.uncertainty = Some("Part of the take clipped, so loudness measurements are less reliable. Lower the input gain a little.".into());
    }
    if let Some((_, msg, goal, drill)) = opp {
        fb.primary_opportunity = Some(msg);
        fb.retry_goal = Some(goal);
        if let Some(d) = exercise(drill) {
            fb.suggestion = Some(Suggestion { exercise_id: d.id.into(), title: d.title.into(), reason: d.objective.into() });
        }
    } else {
        fb.retry_goal = Some("Try the same phrase again and compare the two takes.".into());
    }
    if a.quality.level == QualityLevel::Fair && fb.uncertainty.is_none() {
        fb.uncertainty = Some(format!(
            "Measurements are less certain: {}",
            a.quality.issues.iter().filter(|i| **i != QualityIssue::BackgroundUnknown).map(|i| i.message()).collect::<Vec<_>>().join(" ")
        ));
    }
    if fb.went_well.is_empty() {
        fb.went_well.push("The take was clear enough to analyse.".into());
    }
    fb
}
