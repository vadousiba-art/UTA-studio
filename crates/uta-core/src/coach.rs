//! Vocal Coach: lessons, attempts, adaptive progression and history.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uta_audio::core::SeqNote;
use uta_coach::evaluate::{evaluate, Evaluation, Outcome};
use uta_coach::exercises::{build_plan, exercise, ExercisePlan, GuideNote, VoiceRange, EXERCISES};
use uta_coach::progression::{
    apply, decide, health_status, recommend, schedule_review, session_plan, AttemptSummary, Decision, HealthStatus,
    Recommendation, ReviewState, SessionPlan, SkillProgress,
};
use uta_coach::skills::{is_unlocked, SKILLS};
use uta_project::library::{AttemptEntry, NoteEntry, RecordingEntry, ReviewEntry};

use crate::error::{CoreError, Result};
use crate::settings::SessionLength;
use crate::{now, AppCore, CaptureKind};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillView {
    pub id: String,
    pub stage: u32,
    pub stage_title: String,
    pub title: String,
    pub summary: String,
    pub level: u32,
    pub unlocked: bool,
    pub requires: Vec<String>,
    pub attempts: usize,
    pub last_outcome: Option<String>,
    pub exercises: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewView {
    pub exercise_id: String,
    pub exercise_title: String,
    pub skill_id: String,
    pub due: DateTime<Utc>,
    pub overdue: bool,
    pub box_index: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoachOverview {
    pub skills: Vec<SkillView>,
    pub reviews: Vec<ReviewView>,
    pub health: HealthStatus,
    pub recommendation: Option<Recommendation>,
    pub session: SessionPlan,
    pub recent: Vec<AttemptEntry>,
    pub voice_range: VoiceRange,
    pub voice_range_measured: bool,
    pub current_stage: u32,
    pub disclaimer: String,
    pub practice_days_last_14: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptResult {
    pub attempt: AttemptEntry,
    pub evaluation: Evaluation,
    pub decision: Decision,
    pub decision_text: String,
    pub new_level: u32,
    pub next_review: Option<DateTime<Utc>>,
    pub recording: RecordingEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeResult {
    pub kind: String,
    pub midi: Option<f32>,
    pub note: Option<String>,
    pub voice_range: VoiceRange,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillHistoryPoint {
    pub at: DateTime<Utc>,
    pub level: u32,
    pub outcome: String,
    pub score: Option<f32>,
    pub exercise_id: String,
}

pub(crate) fn to_summary(a: &AttemptEntry) -> Option<AttemptSummary> {
    let outcome = if a.outcome == "stopped" { Outcome::Inconclusive } else { Outcome::parse(&a.outcome)? };
    Some(AttemptSummary {
        at: a.created_at,
        skill: a.skill_id.clone(),
        exercise: a.exercise_id.clone(),
        level: a.level,
        outcome,
        discomfort: a.discomfort,
    })
}

fn seq_notes(notes: &[GuideNote], sr: u32, amp: f32) -> Vec<SeqNote> {
    notes
        .iter()
        .map(|g| SeqNote {
            start: (g.start.max(0.0) * sr as f32) as u64,
            duration: (g.duration.max(0.02) * sr as f32) as u64,
            hz: uta_dsp::tone::midi_to_hz(g.midi),
            amp,
        })
        .collect()
}

impl AppCore {
    fn skill_levels(&self) -> Result<Vec<SkillProgress>> {
        Ok(self
            .library
            .lock()
            .skills()?
            .into_iter()
            .map(|s| SkillProgress { skill: s.skill_id, level: s.level })
            .collect())
    }

    fn all_summaries(&self, days: i64) -> Result<Vec<AttemptSummary>> {
        Ok(self
            .library
            .lock()
            .attempts_since(now() - Duration::days(days))?
            .iter()
            .filter_map(to_summary)
            .collect())
    }

    fn review_tuples(&self) -> Result<Vec<(String, String, DateTime<Utc>)>> {
        Ok(self
            .library
            .lock()
            .reviews()?
            .into_iter()
            .map(|r| (r.exercise_id, r.skill_id, r.due))
            .collect())
    }

    pub fn coach_overview(&self) -> Result<CoachOverview> {
        let levels = self.skill_levels()?;
        let level_of = |id: &str| levels.iter().find(|l| l.skill == id).map(|l| l.level).unwrap_or(0);
        let history = self.all_summaries(30)?;
        let t = now();
        let health = health_status(&history, t);
        let all_attempts = self.library.lock().attempts(None, None, 5000)?;
        let skills = SKILLS
            .iter()
            .map(|s| {
                let mine: Vec<&AttemptEntry> = all_attempts.iter().filter(|a| a.skill_id == s.id).collect();
                SkillView {
                    id: s.id.into(),
                    stage: s.stage as u32,
                    stage_title: s.stage.title().into(),
                    title: s.title.into(),
                    summary: s.summary.into(),
                    level: level_of(s.id),
                    unlocked: is_unlocked(s, level_of, health.clean_window),
                    requires: s
                        .requires
                        .iter()
                        .map(|p| format!("{} level {}", uta_coach::skills::skill(p.skill).map(|d| d.title).unwrap_or(p.skill), p.level))
                        .collect(),
                    attempts: mine.len(),
                    last_outcome: mine.first().map(|a| a.outcome.clone()),
                    exercises: uta_coach::exercises::exercises_for_skill(s.id).map(|e| e.id.to_string()).collect(),
                }
            })
            .collect();
        let reviews_raw = self.library.lock().reviews()?;
        let reviews = reviews_raw
            .iter()
            .map(|r| ReviewView {
                exercise_id: r.exercise_id.clone(),
                exercise_title: exercise(&r.exercise_id).map(|e| e.title.to_string()).unwrap_or_else(|| r.exercise_id.clone()),
                skill_id: r.skill_id.clone(),
                due: r.due,
                overdue: r.due <= t,
                box_index: r.box_index,
            })
            .collect();
        let tuples = self.review_tuples()?;
        let settings = self.settings();
        let mut days: Vec<String> = all_attempts
            .iter()
            .filter(|a| t - a.created_at < Duration::days(14))
            .map(|a| a.created_at.format("%Y-%m-%d").to_string())
            .collect();
        days.sort();
        days.dedup();
        Ok(CoachOverview {
            recommendation: recommend(&levels, &tuples, &health, t),
            session: session_plan(&levels, &tuples, &health, t, settings.session_length == SessionLength::Long),
            current_stage: uta_coach::progression::current_stage(&levels) as u32,
            skills,
            reviews,
            health,
            recent: all_attempts.into_iter().take(20).collect(),
            voice_range: settings.voice_range,
            voice_range_measured: settings.voice_range_measured,
            disclaimer: uta_coach::DISCLAIMER.into(),
            practice_days_last_14: days.len() as u32,
        })
    }

    /// Build the plan for an exercise at the singer's current level for its
    /// skill (or an explicit level).
    pub fn coach_plan(&self, exercise_id: &str, level: Option<u32>) -> Result<ExercisePlan> {
        let def = exercise(exercise_id).ok_or_else(|| CoreError::NotFound(format!("exercise {exercise_id}")))?;
        let levels = self.skill_levels()?;
        let current = levels.iter().find(|l| l.skill == def.skill).map(|l| l.level).unwrap_or(1).max(1);
        let health = health_status(&self.all_summaries(30)?, now());
        if health.resting_until.is_some() && uta_coach::progression::is_demanding(def.skill) {
            return Err(CoreError::Invalid(
                "This exercise is paused while you rest after reporting discomfort. Try a gentle exercise instead.".into(),
            ));
        }
        let skill = uta_coach::skills::skill(def.skill).ok_or_else(|| CoreError::NotFound(def.skill.into()))?;
        let level_of = |id: &str| levels.iter().find(|l| l.skill == id).map(|l| l.level).unwrap_or(0);
        if !is_unlocked(skill, level_of, health.clean_window) && def.kind != uta_coach::exercises::ExerciseKind::Warmup {
            return Err(CoreError::Invalid(format!("“{}” unlocks after: {}", skill.title, skill.requires.iter().map(|p| format!("{} level {}", p.skill, p.level)).collect::<Vec<_>>().join(", "))));
        }
        let seed = self.library.lock().attempts(None, Some(exercise_id), 1000)?.len() as u64;
        Ok(build_plan(def, level.unwrap_or(current), self.settings().voice_range, seed))
    }

    /// Play the demonstration of a plan (with its click).
    pub fn coach_demo(&self, plan: &ExercisePlan) -> Result<()> {
        let amp = self.settings().reference_volume;
        self.with_engine(|e| {
            let sr = e.sample_rate();
            let notes = seq_notes(&plan.demo, sr, amp);
            e.play_sequence(notes, None)?;
            Ok(())
        })
    }

    /// Start recording an attempt; the guide (if any) and the click start on
    /// the same audio frame as the recording.
    pub fn coach_start_attempt(&self, plan: ExercisePlan) -> Result<()> {
        let sr = self.with_engine(|e| Ok(e.sample_rate()))?;
        let amp = self.settings().reference_volume * plan.guide_volume / 0.25;
        let mut notes = seq_notes(&plan.guide, sr, amp);
        // The sequencer also generates the click and stops shortly after its
        // last note, so a silent marker keeps it running to the end.
        let end = (plan.record_seconds * sr as f32) as u64;
        notes.push(SeqNote { start: end.saturating_sub(sr as u64 / 5), duration: 1, hz: 440.0, amp: 0.0 });
        let click = plan.click_bpm;
        self.begin_capture(CaptureKind::Coach(Box::new(plan)), Some((notes, click)))
    }

    /// Stop, grade, save and decide the next level.
    pub fn coach_stop_attempt(&self, user_difficulty: Option<u8>, notes: Option<String>) -> Result<AttemptResult> {
        let (cap, mono, sr) = self.end_capture()?;
        let CaptureKind::Coach(plan) = cap.kind else {
            return Err(CoreError::Invalid("the active recording is not a coach attempt".into()));
        };
        let evaluation = evaluate(&plan, &mono, sr, cap.latency_s);
        self.record_attempt(&plan, evaluation, cap.path, cap.started, mono.len() as f64 / sr as f64, user_difficulty, false, notes.unwrap_or_default())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_attempt(
        &self,
        plan: &ExercisePlan,
        evaluation: Evaluation,
        path: std::path::PathBuf,
        started: DateTime<Utc>,
        duration: f64,
        user_difficulty: Option<u8>,
        discomfort: bool,
        notes: String,
    ) -> Result<AttemptResult> {
        let t = now();
        let lib = self.library.lock();
        let recording = RecordingEntry {
            id: uuid::Uuid::new_v4().to_string(),
            path: path.to_string_lossy().to_string(),
            created_at: started,
            duration,
            source: "coach".into(),
            title: format!("{} (level {})", plan.title, plan.level),
            exercise_id: Some(plan.exercise_id.clone()),
            summary: serde_json::json!({ "outcome": evaluation.outcome.as_str(), "score": evaluation.score }),
        };
        lib.add_recording(&recording)?;
        let history: Vec<AttemptSummary> = lib.attempts(Some(&plan.skill_id), None, 50)?.iter().filter_map(to_summary).collect();
        let current = lib
            .skills()?
            .into_iter()
            .find(|s| s.skill_id == plan.skill_id)
            .map(|s| s.level)
            .unwrap_or(plan.level)
            .max(1);
        let summary = AttemptSummary {
            at: t,
            skill: plan.skill_id.clone(),
            exercise: plan.exercise_id.clone(),
            level: plan.level,
            outcome: evaluation.outcome,
            discomfort,
        };
        let is_warmup = plan.kind == uta_coach::exercises::ExerciseKind::Warmup;
        let decision = if is_warmup { Decision::Keep } else { decide(current, &summary, &history) };
        let new_level = apply(current, decision);
        let attempt = AttemptEntry {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: t,
            exercise_id: plan.exercise_id.clone(),
            skill_id: plan.skill_id.clone(),
            level: plan.level,
            outcome: if discomfort { "stopped".into() } else { evaluation.outcome.as_str().into() },
            score: evaluation.outcome.is_conclusive().then_some(evaluation.score),
            confidence: Some(evaluation.confidence),
            metrics: serde_json::to_value(&evaluation.metrics)?,
            recording_id: Some(recording.id.clone()),
            user_difficulty: user_difficulty.map(|d| d.clamp(1, 5)),
            discomfort,
            notes,
        };
        lib.add_attempt(&attempt)?;
        let mut next_review = None;
        if !is_warmup {
            lib.set_skill(&plan.skill_id, new_level, &serde_json::json!({ "lastDecision": decision }))?;
            let prev = lib.reviews()?.into_iter().find(|r| r.exercise_id == plan.exercise_id).map(|r| ReviewState {
                box_index: r.box_index,
                due: r.due,
                successes: r.successes,
            });
            let st = schedule_review(prev.as_ref(), evaluation.outcome, discomfort, t);
            if evaluation.outcome.is_conclusive() || discomfort || prev.is_some() {
                lib.upsert_review(&ReviewEntry {
                    exercise_id: plan.exercise_id.clone(),
                    skill_id: plan.skill_id.clone(),
                    box_index: st.box_index,
                    due: st.due,
                    last_practiced: t,
                    successes: st.successes,
                    last_outcome: attempt.outcome.clone(),
                })?;
                next_review = Some(st.due);
            }
        }
        Ok(AttemptResult {
            decision_text: decision.explain(),
            attempt,
            evaluation,
            decision,
            new_level,
            next_review,
            recording,
        })
    }

    /// The singer reports pain, throat discomfort or hoarseness: stop all
    /// audio immediately, discard grading, log it and return the health
    /// guidance. Works whether or not a recording is running.
    pub fn coach_report_discomfort(&self, note: Option<String>) -> Result<HealthStatus> {
        let active = self.capture.lock().is_some();
        let _ = self.stop_reference();
        if active {
            let (cap, mono, sr) = self.end_capture()?;
            if let CaptureKind::Coach(plan) = cap.kind {
                let ev = Evaluation {
                    outcome: Outcome::Inconclusive,
                    score: 0.0,
                    confidence: 0.0,
                    checks: vec![],
                    metrics: Default::default(),
                    feedback: Default::default(),
                };
                self.record_attempt(&plan, ev, cap.path, cap.started, mono.len() as f64 / sr as f64, None, true, note.unwrap_or_default())?;
            }
        } else {
            // log a standalone report so the safety rules see it
            let lib = self.library.lock();
            lib.add_attempt(&AttemptEntry {
                id: uuid::Uuid::new_v4().to_string(),
                created_at: now(),
                exercise_id: "self_report".into(),
                skill_id: "health".into(),
                level: 0,
                outcome: "stopped".into(),
                score: None,
                confidence: None,
                metrics: serde_json::json!({}),
                recording_id: None,
                user_difficulty: None,
                discomfort: true,
                notes: note.unwrap_or_default(),
            })?;
        }
        Ok(health_status(&self.all_summaries(30)?, now()))
    }

    /// Range check: record the lowest or highest *comfortable* sustained note.
    pub fn range_start(&self, high: bool) -> Result<()> {
        self.begin_capture(if high { CaptureKind::RangeHigh } else { CaptureKind::RangeLow }, None)
    }

    pub fn range_stop(&self) -> Result<RangeResult> {
        let (cap, mono, sr) = self.end_capture()?;
        let high = match cap.kind {
            CaptureKind::RangeHigh => true,
            CaptureKind::RangeLow => false,
            _ => return Err(CoreError::Invalid("the active recording is not a range check".into())),
        };
        let a = uta_analysis::analyze_phrase(&mono, sr, &Default::default());
        let _ = std::fs::remove_file(&cap.path);
        let mut settings = self.settings();
        let kind = if high { "high" } else { "low" };
        // the longest stable note is the one the singer held
        let held = a.notes.iter().filter(|n| n.duration() >= 0.6 && n.spread_cents < 40.0).max_by(|x, y| x.duration().total_cmp(&y.duration()));
        let Some(n) = held.filter(|_| a.quality.level != uta_analysis::quality::QualityLevel::Poor) else {
            return Ok(RangeResult {
                kind: kind.into(),
                midi: None,
                note: None,
                voice_range: settings.voice_range,
                message: "No steady note of at least 0.6 seconds was found. Hold one easy note for about two seconds.".into(),
            });
        };
        let midi = n.midi.round();
        let mut r = settings.voice_range;
        if high {
            r.high = midi;
        } else {
            r.low = midi;
        }
        if r.high - r.low < 5.0 {
            return Ok(RangeResult {
                kind: kind.into(),
                midi: Some(midi),
                note: Some(uta_dsp::tone::note_name(midi)),
                voice_range: settings.voice_range,
                message: "That note is very close to the other end of your range — check that you sang your lowest and highest comfortable notes.".into(),
            });
        }
        settings.voice_range = r.sanitized();
        settings.voice_range_measured = true;
        let saved = self.update_settings(settings)?;
        Ok(RangeResult {
            kind: kind.into(),
            midi: Some(midi),
            note: Some(uta_dsp::tone::note_name(midi)),
            voice_range: saved.voice_range,
            message: format!(
                "Saved {} as your {} comfortable note. Exercises will stay between {} and {}.",
                uta_dsp::tone::note_name(midi),
                kind,
                uta_dsp::tone::note_name(saved.voice_range.low),
                uta_dsp::tone::note_name(saved.voice_range.high)
            ),
        })
    }

    pub fn skill_history(&self, skill: &str) -> Result<Vec<SkillHistoryPoint>> {
        let mut v: Vec<SkillHistoryPoint> = self
            .library
            .lock()
            .attempts(Some(skill), None, 500)?
            .into_iter()
            .map(|a| SkillHistoryPoint { at: a.created_at, level: a.level, outcome: a.outcome, score: a.score, exercise_id: a.exercise_id })
            .collect();
        v.reverse();
        Ok(v)
    }

    pub fn exercise_library(&self) -> Vec<serde_json::Value> {
        EXERCISES
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id, "skill": e.skill, "kind": e.kind, "title": e.title,
                    "objective": e.objective, "steps": e.steps, "warmup": e.warmup, "safety": e.safety,
                })
            })
            .collect()
    }

    pub fn add_note(&self, text: &str, skill: Option<String>) -> Result<NoteEntry> {
        let t = text.trim();
        if t.is_empty() {
            return Err(CoreError::Invalid("note is empty".into()));
        }
        let n = NoteEntry { id: uuid::Uuid::new_v4().to_string(), created_at: now(), skill_id: skill, text: t.chars().take(4000).collect() };
        self.library.lock().add_note(&n)?;
        Ok(n)
    }

    pub fn notes(&self) -> Result<Vec<NoteEntry>> {
        Ok(self.library.lock().notes(500)?)
    }

    pub fn delete_note(&self, id: &str) -> Result<()> {
        Ok(self.library.lock().delete_note(id)?)
    }
}
