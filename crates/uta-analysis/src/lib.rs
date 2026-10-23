//! Deterministic vocal analysis for UTA Vocal Lab.
//!
//! All measurements here are plain signal processing — no machine learning,
//! no network. Every measurement carries enough context (confidence, voiced
//! ratio, recording quality) that callers can refuse to grade a recording
//! that cannot support a judgement.
//!
//! Main entry points:
//! * [`pitch::PitchTracker`] / [`pitch::track_pitch`] — YIN f0 estimation,
//! * [`live::LivePitch`] — streaming pitch for live meters,
//! * [`phrase::analyze_phrase`] — complete analysis of a recorded phrase,
//! * [`compare`] — comparisons against target notes or a reference take.

pub mod compare;
pub mod dynamics;
pub mod live;
pub mod notes;
pub mod onsets;
pub mod phrase;
pub mod pitch;
pub mod quality;
pub mod spectral;
pub mod stats;
pub mod synth;
pub mod vibrato;

pub use phrase::{analyze_phrase, PhraseAnalysis};
pub use pitch::{PitchConfig, PitchFrame};
pub use uta_dsp::tone::{cents_between, hz_to_midi, midi_to_hz, note_name};
