//! The UTA Vocal Transformation Method: stages, skills and prerequisites.
//!
//! Skills progress independently. A skill becomes available when its
//! prerequisites reach the required *demonstrated* level — never because a
//! number of lessons was completed.

use serde::Serialize;

pub const MAX_LEVEL: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Foundation = 1,
    Control = 2,
    ExpressiveCharacter = 3,
    AdvancedStyle = 4,
    JapaneseSong = 5,
    PerformanceProduction = 6,
}

impl Stage {
    pub fn title(self) -> &'static str {
        match self {
            Stage::Foundation => "Foundation",
            Stage::Control => "Control",
            Stage::ExpressiveCharacter => "Expressive Character",
            Stage::AdvancedStyle => "Advanced Style",
            Stage::JapaneseSong => "Japanese Song Application",
            Stage::PerformanceProduction => "Performance and Production",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prerequisite {
    pub skill: &'static str,
    pub level: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDef {
    pub id: &'static str,
    pub stage: Stage,
    pub title: &'static str,
    pub summary: &'static str,
    pub requires: &'static [Prerequisite],
    /// Requires a recent discomfort-free history before unlocking.
    pub needs_clean_health_window: bool,
}

const fn pre(skill: &'static str, level: u32) -> Prerequisite {
    Prerequisite { skill, level }
}

pub static SKILLS: &[SkillDef] = &[
    // Stage 1 — Foundation
    SkillDef { id: "pitch_matching", stage: Stage::Foundation, title: "Pitch matching", summary: "Hear a note and sing it back, comfortably and accurately.", requires: &[], needs_clean_health_window: false },
    SkillDef { id: "pitch_accuracy", stage: Stage::Foundation, title: "Scale accuracy", summary: "Sing short scales with each note in tune.", requires: &[pre("pitch_matching", 1)], needs_clean_health_window: false },
    SkillDef { id: "gentle_onset", stage: Stage::Foundation, title: "Gentle onset", summary: "Start notes cleanly — no scoop from below, no hard 'click'.", requires: &[], needs_clean_health_window: false },
    SkillDef { id: "vowel_clarity", stage: Stage::Foundation, title: "Vowel clarity", summary: "Hold the five Japanese vowels あ い う え お on one steady pitch.", requires: &[pre("pitch_matching", 1)], needs_clean_health_window: false },
    SkillDef { id: "rhythm", stage: Stage::Foundation, title: "Rhythm", summary: "Start syllables exactly with the click.", requires: &[], needs_clean_health_window: false },
    // Stage 2 — Control
    SkillDef { id: "sustain_stability", stage: Stage::Control, title: "Sustained-note stability", summary: "Hold a note without drifting or wobbling.", requires: &[pre("pitch_matching", 2)], needs_clean_health_window: false },
    SkillDef { id: "phrase_length", stage: Stage::Control, title: "Breath phrasing", summary: "Sustain an easy, even tone for longer on one breath.", requires: &[pre("sustain_stability", 1)], needs_clean_health_window: false },
    SkillDef { id: "smooth_transitions", stage: Stage::Control, title: "Smooth transitions", summary: "Glide between notes without breaks or jumps.", requires: &[pre("pitch_accuracy", 2)], needs_clean_health_window: false },
    SkillDef { id: "register_coordination", stage: Stage::Control, title: "Register coordination", summary: "Slide through your range with an even sound — no sudden flips.", requires: &[pre("smooth_transitions", 1)], needs_clean_health_window: false },
    SkillDef { id: "articulation", stage: Stage::Control, title: "Consistent articulation", summary: "Repeat syllables evenly in time and level.", requires: &[pre("rhythm", 2)], needs_clean_health_window: false },
    // Stage 3 — Expressive character
    SkillDef { id: "dynamic_contrast", stage: Stage::ExpressiveCharacter, title: "Soft-to-strong contrast", summary: "Swell from soft to fuller and back while keeping the pitch.", requires: &[pre("sustain_stability", 2)], needs_clean_health_window: false },
    SkillDef { id: "resonance_choices", stage: Stage::ExpressiveCharacter, title: "Resonance colour", summary: "Sing the same note with a darker and a brighter colour.", requires: &[pre("vowel_clarity", 2), pre("sustain_stability", 2)], needs_clean_health_window: false },
    SkillDef { id: "dramatic_phrasing", stage: Stage::ExpressiveCharacter, title: "Dramatic phrasing", summary: "Shape a phrase with a planned build and release.", requires: &[pre("dynamic_contrast", 1), pre("pitch_accuracy", 2)], needs_clean_health_window: false },
    SkillDef { id: "rhythmic_precision", stage: Stage::ExpressiveCharacter, title: "Rhythmic precision", summary: "Faster and syncopated rhythms, still on time.", requires: &[pre("rhythm", 3)], needs_clean_health_window: false },
    // Stage 4 — Advanced style
    SkillDef { id: "fast_transitions", stage: Stage::AdvancedStyle, title: "Fast register transitions", summary: "Quick arpeggios and leaps that land in tune.", requires: &[pre("register_coordination", 2), pre("pitch_accuracy", 3)], needs_clean_health_window: false },
    SkillDef { id: "ornamentation", stage: Stage::AdvancedStyle, title: "Ornamentation and vibrato", summary: "Add controlled vibrato on purpose — and leave it out on purpose.", requires: &[pre("sustain_stability", 3)], needs_clean_health_window: false },
    SkillDef { id: "stylistic_texture", stage: Stage::AdvancedStyle, title: "Guided stylistic texture", summary: "Introductory, low-volume texture work (e.g. edge, gentle grit). Stop at any discomfort.", requires: &[pre("sustain_stability", 3), pre("dynamic_contrast", 2), pre("register_coordination", 2)], needs_clean_health_window: true },
    // Stage 5 — Japanese song application
    SkillDef { id: "mora_timing", stage: Stage::JapaneseSong, title: "Mora timing", summary: "Give every mora (拍) its own even beat, including っ and ん.", requires: &[pre("rhythm", 2), pre("vowel_clarity", 1)], needs_clean_health_window: false },
    SkillDef { id: "japanese_vowels", stage: Stage::JapaneseSong, title: "Japanese vowels in melody", summary: "Keep pure Japanese vowels while the melody moves.", requires: &[pre("vowel_clarity", 2), pre("pitch_accuracy", 2)], needs_clean_health_window: false },
    SkillDef { id: "lyric_phrase", stage: Stage::JapaneseSong, title: "Lyric phrase alignment", summary: "Sing song phrases in time and in tune with the reference (Song Lab).", requires: &[pre("mora_timing", 1), pre("pitch_accuracy", 2)], needs_clean_health_window: false },
    // Stage 6 — Performance and production
    SkillDef { id: "full_song", stage: Stage::PerformanceProduction, title: "Full-song practice", summary: "Record complete takes, compare them and build a comp (Studio).", requires: &[pre("lyric_phrase", 2)], needs_clean_health_window: false },
    SkillDef { id: "production", stage: Stage::PerformanceProduction, title: "Doubles, harmonies and mixing", summary: "Layer doubles and harmonies, then mix and export (Studio).", requires: &[pre("full_song", 1)], needs_clean_health_window: false },
];

pub fn skill(id: &str) -> Option<&'static SkillDef> {
    SKILLS.iter().find(|s| s.id == id)
}

/// Whether `skill` is unlocked given current levels (0 = never practised).
pub fn is_unlocked(def: &SkillDef, level_of: impl Fn(&str) -> u32, clean_health_window: bool) -> bool {
    if def.needs_clean_health_window && !clean_health_window {
        return false;
    }
    def.requires.iter().all(|p| level_of(p.skill) >= p.level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerequisites_reference_real_skills_without_cycles() {
        for s in SKILLS {
            for p in s.requires {
                let dep = skill(p.skill).unwrap_or_else(|| panic!("{} requires unknown {}", s.id, p.skill));
                assert!(dep.stage <= s.stage, "{} depends on a later-stage skill", s.id);
                assert!(p.level >= 1 && p.level <= MAX_LEVEL);
            }
        }
        // every skill is reachable: iterate unlocking with max levels
        let mut unlocked: Vec<&str> = Vec::new();
        for _ in 0..SKILLS.len() {
            for s in SKILLS {
                if !unlocked.contains(&s.id) && is_unlocked(s, |id| if unlocked.contains(&id) { MAX_LEVEL } else { 0 }, true) {
                    unlocked.push(s.id);
                }
            }
        }
        assert_eq!(unlocked.len(), SKILLS.len());
    }

    #[test]
    fn texture_needs_clean_health_window() {
        let t = skill("stylistic_texture").unwrap();
        assert!(!is_unlocked(t, |_| MAX_LEVEL, false));
        assert!(is_unlocked(t, |_| MAX_LEVEL, true));
        assert!(!is_unlocked(t, |_| 1, true));
    }
}
