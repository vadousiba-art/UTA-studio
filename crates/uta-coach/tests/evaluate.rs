//! Grading tests with synthetic "singers".

use uta_analysis::synth;
use uta_coach::evaluate::{evaluate, Outcome};
use uta_coach::exercises::{build_plan, exercise, ExercisePlan, VoiceRange};

const SR: u32 = 48_000;

/// Render the plan's targets as a sung signal: each target note shifted by
/// `cents`, delayed by `delay` seconds, with low room tone.
fn sing(plan: &ExercisePlan, cents: f32, delay: f32) -> Vec<f32> {
    let n = (plan.record_seconds * SR as f32) as usize;
    let mut out = synth::noise(0.0003, plan.record_seconds, SR, 11);
    for t in &plan.targets {
        let hz = uta_dsp::tone::midi_to_hz(t.midi) * 2f32.powf(cents / 1200.0);
        let tone = synth::harmonic_tone(hz, &[1.0, 0.6, 0.3, 0.15], t.duration * 0.98, SR);
        let start = ((t.start + delay) * SR as f32) as usize;
        let fade = 480;
        let len = tone.len();
        for (k, v) in tone.iter().enumerate() {
            let g = (k.min(len - 1 - k) as f32 / fade as f32).min(1.0);
            if start + k < n {
                out[start + k] += v * 0.35 * g;
            }
        }
    }
    out
}

fn plan(id: &str, level: u32) -> ExercisePlan {
    build_plan(exercise(id).unwrap(), level, VoiceRange::default(), 3)
}

#[test]
fn accurate_pitch_match_succeeds() {
    let p = plan("pitch_match", 2);
    let e = evaluate(&p, &sing(&p, 5.0, 0.03), SR, 0.03);
    assert_eq!(e.outcome, Outcome::Success, "{:#?}", e.feedback);
    assert!(e.feedback.primary_opportunity.is_none());
    assert!(e.confidence > 0.8);
    assert!(e.metrics["meanAbsErrorCents"] < 10.0);
}

#[test]
fn flat_pitch_match_fails_with_specific_feedback() {
    let p = plan("pitch_match", 3); // tolerance 25 cents
    let e = evaluate(&p, &sing(&p, -70.0, 0.0), SR, 0.0);
    assert_ne!(e.outcome, Outcome::Success);
    assert_ne!(e.outcome, Outcome::Inconclusive);
    let opp = e.feedback.primary_opportunity.unwrap();
    assert!(opp.contains("flat"), "{opp}");
    assert!(e.feedback.retry_goal.unwrap().contains("25"));
    assert!(e.feedback.suggestion.is_some());
}

#[test]
fn octave_lower_counts_as_match() {
    let p = plan("pitch_match", 1);
    let e = evaluate(&p, &sing(&p, -1200.0, 0.0), SR, 0.0);
    assert_eq!(e.outcome, Outcome::Success, "{:#?}", e);
    assert!(e.feedback.went_well.iter().any(|w| w.contains("octave")));
}

#[test]
fn silence_and_noise_are_inconclusive_not_graded() {
    let p = plan("scale_5", 2);
    let silent = vec![0.0f32; (p.record_seconds * SR as f32) as usize];
    let e = evaluate(&p, &silent, SR, 0.0);
    assert_eq!(e.outcome, Outcome::Inconclusive);
    assert!(e.feedback.uncertainty.is_some());
    assert!(e.checks.is_empty(), "no grades on unmeasurable audio");

    let noise = synth::noise(0.3, p.record_seconds, SR, 4);
    let e = evaluate(&p, &noise, SR, 0.0);
    assert_eq!(e.outcome, Outcome::Inconclusive, "{:#?}", e.feedback);

    // singing buried in loud noise
    let mut sung = sing(&p, 0.0, 0.0);
    let n = synth::noise(0.2, p.record_seconds, SR, 8);
    for (s, x) in sung.iter_mut().zip(&n) {
        *s += x;
    }
    let e = evaluate(&p, &sung, SR, 0.0);
    assert_eq!(e.outcome, Outcome::Inconclusive, "{:?}", e.feedback);
}

#[test]
fn clipped_recording_is_inconclusive() {
    let p = plan("sustain_hold", 1);
    let s: Vec<f32> = sing(&p, 0.0, 0.0).iter().map(|v| (v * 10.0).clamp(-1.0, 1.0)).collect();
    let e = evaluate(&p, &s, SR, 0.0);
    assert_eq!(e.outcome, Outcome::Inconclusive);
    assert!(e.feedback.uncertainty.unwrap().to_lowercase().contains("clip"));
}

#[test]
fn scale_success_and_partial() {
    let p = plan("scale_5", 2);
    let e = evaluate(&p, &sing(&p, 8.0, 0.02), SR, 0.02);
    assert_eq!(e.outcome, Outcome::Success, "{:#?}", e.checks);
    let e = evaluate(&p, &sing(&p, 60.0, 0.0), SR, 0.0);
    assert!(matches!(e.outcome, Outcome::NeedsWork | Outcome::Partial), "{:?}", e.outcome);
    assert!(e.feedback.primary_opportunity.unwrap().contains("sharp"));
}

#[test]
fn sustain_detects_drift() {
    let p = plan("sustain_hold", 2); // 5 s, max drift 30 cents
    let t = p.targets[0];
    let n = (p.record_seconds * SR as f32) as usize;
    let mut s = vec![0.0f32; n];
    let tone = synth::drifting_tone(uta_dsp::tone::midi_to_hz(t.midi), -80.0, t.duration, SR);
    let start = (t.start * SR as f32) as usize;
    for (k, v) in tone.iter().enumerate() {
        s[start + k] = v * 0.5;
    }
    let room = synth::noise(0.0003, p.record_seconds, SR, 5);
    for (a, b) in s.iter_mut().zip(&room) {
        *a += b;
    }
    let e = evaluate(&p, &s, SR, 0.0);
    assert_ne!(e.outcome, Outcome::Success, "{:#?}", e.checks);
    let drift = e.checks.iter().find(|c| c.key == "drift").unwrap();
    assert!(!drift.pass);
    assert!(e.metrics["driftCents"] < -50.0);
}

#[test]
fn rhythm_on_time_vs_rushing() {
    let p = plan("rhythm_ta", 1);
    let make = |offset: f32| {
        let mut s = synth::noise(0.0005, p.record_seconds, SR, 2);
        let tone = synth::harmonic_tone(160.0, &[1.0, 0.5, 0.3], 0.12, SR);
        for b in &p.beats {
            let st = ((b + offset) * SR as f32) as usize;
            for (k, v) in tone.iter().enumerate() {
                let att = (k as f32 / 240.0).min(1.0);
                if st + k < s.len() {
                    s[st + k] += v * 0.4 * att;
                }
            }
        }
        s
    };
    let e = evaluate(&p, &make(0.01), SR, 0.0);
    assert_eq!(e.outcome, Outcome::Success, "{:#?}", e.checks);
    let e = evaluate(&p, &make(-0.12), SR, 0.0);
    assert_ne!(e.outcome, Outcome::Success);
    assert!(e.feedback.primary_opportunity.unwrap().contains("rushing"));
    // latency compensation: singer is "late" only by the measured latency
    let e = evaluate(&p, &make(0.09), SR, 0.08);
    assert_eq!(e.outcome, Outcome::Success, "{:#?}", e.checks);
}

#[test]
fn every_exercise_evaluates_without_panicking_on_typical_and_bad_input() {
    for def in uta_coach::exercises::EXERCISES {
        for level in [1, 3, 5] {
            let p = build_plan(def, level, VoiceRange::default(), 1);
            let n = (p.record_seconds * SR as f32) as usize;
            for sig in [sing(&p, 0.0, 0.0), vec![0.0; n], synth::noise(0.2, p.record_seconds, SR, 1), vec![]] {
                let e = evaluate(&p, &sig, SR, 0.02);
                if e.outcome != Outcome::Inconclusive {
                    assert!(!e.checks.is_empty(), "{} graded without checks", def.id);
                    assert!((0.0..=1.0).contains(&e.score));
                }
                assert!(!e.feedback.headline.is_empty() || e.outcome == Outcome::Inconclusive);
            }
        }
    }
}

#[test]
fn swell_measures_contrast_and_warns_against_excess() {
    let p = plan("swell_messa", 1);
    let t = p.targets[0];
    let n = (p.record_seconds * SR as f32) as usize;
    let mut s = synth::noise(0.0003, p.record_seconds, SR, 6);
    let tone = synth::harmonic_tone(uta_dsp::tone::midi_to_hz(t.midi), &[1.0, 0.5, 0.2], t.duration, SR);
    let start = (t.start * SR as f32) as usize;
    let len = tone.len();
    for (k, v) in tone.iter().enumerate() {
        let x = k as f32 / len as f32;
        // soft (0.1) → full (0.5) → soft: ~14 dB swell
        let g = 0.1 + 0.4 * (1.0 - (2.0 * x - 1.0).abs());
        if start + k < n {
            s[start + k] += v * g * 0.6;
        }
    }
    let e = evaluate(&p, &s, SR, 0.0);
    assert_eq!(e.outcome, Outcome::Success, "{:#?}", e.checks);
    // thirds-averaged contrast of a linear 0.1→0.5→0.1 swell is ≈7 dB
    assert!(e.metrics["contrastDb"] > 5.0, "{}", e.metrics["contrastDb"]);
    assert!(e.feedback.safety_reminder.is_some());
}
