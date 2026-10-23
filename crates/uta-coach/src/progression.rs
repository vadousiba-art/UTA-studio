//! Adaptive progression, spaced review and safety rules.
//!
//! All rules are explicit and documented in docs/training-method.md:
//!
//! * **Level up**: the last [`STREAK`] conclusive attempts at the current
//!   level were all `Success`.
//! * **Level down / simplify**: 2 of the last 3 conclusive attempts at the
//!   current level were `NeedsWork`.
//! * **Keep**: anything else (mixed results).
//! * **Clean recording request**: 2 inconclusive attempts in a row.
//! * **Stop**: the singer reports pain, throat discomfort or hoarseness —
//!   the exercise halts immediately, rest is recommended, and demanding
//!   skills stay locked for [`REST_DAYS_AFTER_DISCOMFORT`] days. Repeated
//!   reports recommend seeing a qualified voice teacher or medical
//!   professional.
//!
//! Lesson *counts* never unlock anything.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::evaluate::Outcome;
use crate::skills::{is_unlocked, Stage, MAX_LEVEL, SKILLS};

pub const STREAK: usize = 3;
pub const REST_DAYS_AFTER_DISCOMFORT: i64 = 2;
/// Discomfort reports within this window block texture work.
pub const HEALTH_WINDOW_DAYS: i64 = 14;
/// Leitner intervals in days per box.
pub const REVIEW_INTERVALS: [i64; 6] = [1, 2, 4, 8, 16, 32];

/// A past attempt as needed by the rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptSummary {
    pub at: DateTime<Utc>,
    pub skill: String,
    pub exercise: String,
    pub level: u32,
    pub outcome: Outcome,
    pub discomfort: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Decision {
    LevelUp { to: u32 },
    Keep,
    Simplify { to: u32 },
    RequestCleanRecording,
    Stop,
}

impl Decision {
    pub fn explain(&self) -> String {
        match self {
            Decision::LevelUp { to } => format!("{STREAK} successful attempts in a row — moving up to level {to}."),
            Decision::Keep => "Mixed results so far — staying at this level to build consistency.".into(),
            Decision::Simplify { to } => format!("This level is a stretch right now — switching to level {to} (slower / simpler) to rebuild confidence."),
            Decision::RequestCleanRecording => "The last recordings couldn't be measured. Check the microphone level and background noise before trying again.".into(),
            Decision::Stop => "You reported discomfort. The exercise has stopped — rest your voice.".into(),
        }
    }
}

/// Decide what happens after a new attempt. `history` holds earlier attempts
/// for this skill, newest first (the new attempt is *not* included).
pub fn decide(current_level: u32, new: &AttemptSummary, history: &[AttemptSummary]) -> Decision {
    if new.discomfort {
        return Decision::Stop;
    }
    let level = current_level.clamp(1, MAX_LEVEL);
    if new.outcome == Outcome::Inconclusive {
        let prev_inconclusive = history.first().map(|h| h.outcome == Outcome::Inconclusive).unwrap_or(false);
        return if prev_inconclusive { Decision::RequestCleanRecording } else { Decision::Keep };
    }
    let mut recent: Vec<Outcome> = vec![new.outcome];
    recent.extend(
        history
            .iter()
            .filter(|h| h.level == level && h.outcome.is_conclusive() && !h.discomfort)
            .map(|h| h.outcome),
    );
    if recent.len() >= STREAK && recent[..STREAK].iter().all(|o| *o == Outcome::Success) {
        return if level < MAX_LEVEL { Decision::LevelUp { to: level + 1 } } else { Decision::Keep };
    }
    let last3 = &recent[..recent.len().min(3)];
    if last3.len() >= 2 && last3.iter().filter(|o| **o == Outcome::NeedsWork).count() >= 2 {
        return if level > 1 { Decision::Simplify { to: level - 1 } } else { Decision::Keep };
    }
    Decision::Keep
}

pub fn apply(level: u32, d: Decision) -> u32 {
    match d {
        Decision::LevelUp { to } | Decision::Simplify { to } => to,
        _ => level.max(1),
    }
}

/// Spaced-review state for one exercise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewState {
    pub box_index: u32,
    pub due: DateTime<Utc>,
    pub successes: u32,
}

/// Leitner scheduling. Inconclusive and stopped attempts don't move the box.
pub fn schedule_review(prev: Option<&ReviewState>, outcome: Outcome, discomfort: bool, now: DateTime<Utc>) -> ReviewState {
    let mut s = prev.cloned().unwrap_or(ReviewState { box_index: 0, due: now, successes: 0 });
    if discomfort {
        s.due = now + Duration::days(REST_DAYS_AFTER_DISCOMFORT);
        return s;
    }
    match outcome {
        Outcome::Success => {
            s.successes += 1;
            s.box_index = (s.box_index + 1).min(REVIEW_INTERVALS.len() as u32 - 1);
        }
        Outcome::Partial => {}
        Outcome::NeedsWork => s.box_index = 0,
        Outcome::Inconclusive => return s,
    }
    s.due = now + Duration::days(REVIEW_INTERVALS[s.box_index as usize]);
    s
}

/// Health summary derived from self-reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthStatus {
    pub resting_until: Option<DateTime<Utc>>,
    pub reports_last_14_days: u32,
    /// No discomfort reported in the health window.
    pub clean_window: bool,
    pub recommend_professional: bool,
    pub message: Option<String>,
}

pub fn health_status(history: &[AttemptSummary], now: DateTime<Utc>) -> HealthStatus {
    let recent: Vec<&AttemptSummary> = history
        .iter()
        .filter(|a| a.discomfort && now - a.at < Duration::days(HEALTH_WINDOW_DAYS))
        .collect();
    let last = recent.iter().map(|a| a.at).max();
    let resting_until = last.map(|t| t + Duration::days(REST_DAYS_AFTER_DISCOMFORT)).filter(|t| *t > now);
    // distinct days with reports
    let mut days: Vec<String> = recent.iter().map(|a| a.at.format("%Y-%m-%d").to_string()).collect();
    days.sort();
    days.dedup();
    let recommend = days.len() >= 2;
    let message = if recommend {
        Some("You've reported discomfort on more than one day recently. Please rest and talk to a qualified voice teacher or a medical professional (for example an ENT doctor) before intensive practice. This app can't assess vocal health.".into())
    } else if resting_until.is_some() {
        Some("You reported discomfort recently. Gentle, short sessions only — stop if anything hurts. Demanding exercises are paused for a couple of days.".into())
    } else {
        None
    };
    HealthStatus {
        resting_until,
        reports_last_14_days: recent.len() as u32,
        clean_window: recent.is_empty(),
        recommend_professional: recommend,
        message,
    }
}

/// Skills paused while resting (demanding work).
pub fn is_demanding(skill: &str) -> bool {
    matches!(skill, "dynamic_contrast" | "stylistic_texture" | "fast_transitions" | "register_coordination" | "dramatic_phrasing" | "phrase_length")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillProgress {
    pub skill: String,
    pub level: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub skill: String,
    pub exercise: String,
    pub level: u32,
    pub reason: String,
}

/// Pick the single next exercise:
/// 1. an overdue review (oldest first), unless paused for rest;
/// 2. otherwise the unlocked skill with the lowest level in the earliest
///    stage that is below level 3;
/// 3. otherwise the lowest-level unlocked skill overall.
pub fn recommend(
    levels: &[SkillProgress],
    reviews: &[(String, String, DateTime<Utc>)], // (exercise, skill, due)
    health: &HealthStatus,
    now: DateTime<Utc>,
) -> Option<Recommendation> {
    let level_of = |id: &str| levels.iter().find(|l| l.skill == id).map(|l| l.level).unwrap_or(0);
    let resting = health.resting_until.is_some();
    let allowed = |skill: &str| !(resting && is_demanding(skill));
    let mut due: Vec<&(String, String, DateTime<Utc>)> = reviews.iter().filter(|r| r.2 <= now && allowed(&r.1)).collect();
    due.sort_by_key(|r| r.2);
    if let Some((ex, skill, at)) = due.first().map(|r| (&r.0, &r.1, r.2)) {
        let days = (now - at).num_days();
        return Some(Recommendation {
            skill: skill.clone(),
            exercise: ex.clone(),
            level: level_of(skill).max(1),
            reason: if days > 0 { format!("Review due {days} day{} ago — a quick check keeps it solid.", if days == 1 { "" } else { "s" }) } else { "Review due today — a quick check keeps it solid.".into() },
        });
    }
    let unlocked: Vec<&crate::skills::SkillDef> = SKILLS
        .iter()
        .filter(|s| is_unlocked(s, level_of, health.clean_window))
        .filter(|s| allowed(s.id))
        .filter(|s| crate::exercises::exercises_for_skill(s.id).next().is_some())
        .collect();
    let pick = unlocked
        .iter()
        .filter(|s| level_of(s.id) < 3)
        .min_by_key(|s| (s.stage as u32, level_of(s.id)))
        .or_else(|| unlocked.iter().min_by_key(|s| (level_of(s.id), s.stage as u32)))?;
    let ex = crate::exercises::exercises_for_skill(pick.id).next()?;
    let lvl = level_of(pick.id);
    Some(Recommendation {
        skill: pick.id.into(),
        exercise: ex.id.into(),
        level: lvl.max(1),
        reason: if lvl == 0 {
            format!("Next step in {}: {}", pick.stage.title(), pick.summary)
        } else {
            format!("Build {} (level {lvl}): {}", pick.title.to_lowercase(), pick.summary)
        },
    })
}

/// A short or long practice session plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPlan {
    pub minutes: u32,
    pub items: Vec<Recommendation>,
}

pub fn session_plan(
    levels: &[SkillProgress],
    reviews: &[(String, String, DateTime<Utc>)],
    health: &HealthStatus,
    now: DateTime<Utc>,
    long: bool,
) -> SessionPlan {
    let mut items = vec![Recommendation {
        skill: "smooth_transitions".into(),
        exercise: "warmup_siren".into(),
        level: 1,
        reason: "Start gently: a soft siren warms the voice up.".into(),
    }];
    let count = if long { 4 } else { 2 };
    let mut lv = levels.to_vec();
    let mut rv = reviews.to_vec();
    for _ in 0..count {
        let Some(r) = recommend(&lv, &rv, health, now) else { break };
        if items.iter().any(|i| i.exercise == r.exercise) {
            // avoid repeating: pretend this skill is a level higher for selection
            if let Some(l) = lv.iter_mut().find(|l| l.skill == r.skill) {
                l.level += 3;
            } else {
                lv.push(SkillProgress { skill: r.skill.clone(), level: 3 });
            }
            rv.retain(|x| x.0 != r.exercise);
            continue;
        }
        rv.retain(|x| x.0 != r.exercise);
        if let Some(l) = lv.iter_mut().find(|l| l.skill == r.skill) {
            l.level += 3;
        } else {
            lv.push(SkillProgress { skill: r.skill.clone(), level: 3 });
        }
        items.push(r);
    }
    SessionPlan { minutes: if long { 25 } else { 10 }, items }
}

/// Stage reached = highest stage with at least one skill at level ≥ 1.
pub fn current_stage(levels: &[SkillProgress]) -> Stage {
    SKILLS
        .iter()
        .filter(|s| levels.iter().any(|l| l.skill == s.id && l.level >= 1))
        .map(|s| s.stage)
        .max()
        .unwrap_or(Stage::Foundation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att(level: u32, outcome: Outcome) -> AttemptSummary {
        AttemptSummary { at: Utc::now(), skill: "pitch_matching".into(), exercise: "pitch_match".into(), level, outcome, discomfort: false }
    }

    #[test]
    fn three_successes_level_up() {
        let hist = vec![att(2, Outcome::Success), att(2, Outcome::Success)];
        assert_eq!(decide(2, &att(2, Outcome::Success), &hist), Decision::LevelUp { to: 3 });
        // inconclusive attempts in between don't break or count toward the streak
        let hist = vec![att(2, Outcome::Inconclusive), att(2, Outcome::Success), att(2, Outcome::Success)];
        assert_eq!(decide(2, &att(2, Outcome::Success), &hist), Decision::LevelUp { to: 3 });
        // successes at a lower level don't count
        let hist = vec![att(1, Outcome::Success), att(1, Outcome::Success)];
        assert_eq!(decide(2, &att(2, Outcome::Success), &hist), Decision::Keep);
        // capped at max level
        let hist = vec![att(5, Outcome::Success), att(5, Outcome::Success)];
        assert_eq!(decide(5, &att(5, Outcome::Success), &hist), Decision::Keep);
    }

    #[test]
    fn mixed_results_keep_level() {
        let hist = vec![att(3, Outcome::Success), att(3, Outcome::NeedsWork)];
        assert_eq!(decide(3, &att(3, Outcome::Partial), &hist), Decision::Keep);
        let hist = vec![att(3, Outcome::Success), att(3, Outcome::Success)];
        assert_eq!(decide(3, &att(3, Outcome::Partial), &hist), Decision::Keep);
    }

    #[test]
    fn repeated_difficulty_simplifies() {
        let hist = vec![att(3, Outcome::NeedsWork), att(3, Outcome::Success)];
        assert_eq!(decide(3, &att(3, Outcome::NeedsWork), &hist), Decision::Simplify { to: 2 });
        let hist = vec![att(1, Outcome::NeedsWork)];
        assert_eq!(decide(1, &att(1, Outcome::NeedsWork), &hist), Decision::Keep, "level 1 can't go lower");
    }

    #[test]
    fn low_confidence_never_grades_and_requests_clean_recording() {
        assert_eq!(decide(2, &att(2, Outcome::Inconclusive), &[]), Decision::Keep);
        let hist = vec![att(2, Outcome::Inconclusive)];
        assert_eq!(decide(2, &att(2, Outcome::Inconclusive), &hist), Decision::RequestCleanRecording);
        // and the review box does not move
        let now = Utc::now();
        let prev = ReviewState { box_index: 3, due: now, successes: 5 };
        let s = schedule_review(Some(&prev), Outcome::Inconclusive, false, now);
        assert_eq!(s, prev);
    }

    #[test]
    fn discomfort_halts_and_pauses_demanding_work() {
        let mut a = att(2, Outcome::Success);
        a.discomfort = true;
        assert_eq!(decide(2, &a, &[]), Decision::Stop);
        let now = Utc::now();
        let h = health_status(&[a.clone()], now);
        assert!(h.resting_until.is_some());
        assert!(!h.clean_window);
        assert!(!h.recommend_professional);
        let levels = vec![SkillProgress { skill: "sustain_stability".into(), level: 5 }];
        let reviews = vec![("swell_messa".to_string(), "dynamic_contrast".to_string(), now - Duration::days(3))];
        let r = recommend(&levels, &reviews, &h, now).unwrap();
        assert_ne!(r.skill, "dynamic_contrast", "demanding review is skipped while resting");
        // two separate days → recommend a professional
        let mut b = a.clone();
        b.at = now - Duration::days(3);
        let h2 = health_status(&[a, b], now);
        assert!(h2.recommend_professional);
        assert!(h2.message.unwrap().contains("medical professional"));
    }

    #[test]
    fn review_scheduling() {
        let now = Utc::now();
        let s1 = schedule_review(None, Outcome::Success, false, now);
        assert_eq!(s1.box_index, 1);
        assert_eq!((s1.due - now).num_days(), 2);
        let s2 = schedule_review(Some(&s1), Outcome::Success, false, now);
        assert_eq!((s2.due - now).num_days(), 4);
        let s3 = schedule_review(Some(&s2), Outcome::Partial, false, now);
        assert_eq!(s3.box_index, 2);
        let s4 = schedule_review(Some(&s3), Outcome::NeedsWork, false, now);
        assert_eq!(s4.box_index, 0);
        assert_eq!((s4.due - now).num_days(), 1);
        let s5 = schedule_review(Some(&s2), Outcome::Success, true, now);
        assert_eq!(s5.box_index, s2.box_index, "discomfort doesn't promote");
    }

    #[test]
    fn beginner_gets_foundation_exercise_and_overdue_review_wins() {
        let now = Utc::now();
        let h = health_status(&[], now);
        let r = recommend(&[], &[], &h, now).unwrap();
        assert_eq!(crate::skills::skill(&r.skill).unwrap().stage, Stage::Foundation);
        let reviews = vec![("scale_5".to_string(), "pitch_accuracy".to_string(), now - Duration::days(2))];
        let r = recommend(&[], &reviews, &h, now).unwrap();
        assert_eq!(r.exercise, "scale_5");
        assert!(r.reason.contains("2 days"));
    }

    #[test]
    fn advanced_skills_unlock_by_level_not_count() {
        let now = Utc::now();
        let h = health_status(&[], now);
        // lots of level-1 skills (many lessons) don't unlock stage 3
        let levels: Vec<SkillProgress> = ["pitch_matching", "pitch_accuracy", "gentle_onset", "vowel_clarity", "rhythm"]
            .iter()
            .map(|s| SkillProgress { skill: s.to_string(), level: 1 })
            .collect();
        let plan = session_plan(&levels, &[], &h, now, true);
        for it in &plan.items {
            let st = crate::skills::skill(&it.skill).unwrap().stage;
            assert!(st <= Stage::Control, "{} unlocked too early", it.skill);
        }
        assert_eq!(plan.items[0].exercise, "warmup_siren");
        assert!(plan.items.len() >= 3);
        // unique exercises
        let mut ids: Vec<&str> = plan.items.iter().map(|i| i.exercise.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), plan.items.len());
    }
}
