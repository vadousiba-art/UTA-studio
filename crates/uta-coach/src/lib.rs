//! Layer A of the coaching system: a deterministic, offline coach.
//!
//! * [`skills`] — the six-stage UTA Vocal Transformation Method.
//! * [`exercises`] — exercise library and plan generation in the singer's
//!   comfortable range.
//! * [`evaluate`] — grading from measured evidence with honest uncertainty.
//! * [`progression`] — adaptive difficulty, spaced review, safety.
//!
//! This crate never uses a language model and never needs a network
//! connection. Optional explanation layers (local LLMs) may only rephrase its
//! structured output; they are not part of this crate.

pub mod evaluate;
pub mod exercises;
pub mod progression;
pub mod skills;

/// Shown wherever coaching appears.
pub const DISCLAIMER: &str = "UTA Vocal Lab gives practice feedback from audio measurements. It is not a substitute for a qualified voice teacher or medical professional, and it cannot assess vocal health. Stop and rest if singing causes pain, discomfort or hoarseness.";
