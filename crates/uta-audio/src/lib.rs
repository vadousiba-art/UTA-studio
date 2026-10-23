//! Audio engine for UTA Vocal Lab.
//!
//! Thread model (see docs/architecture.md):
//!
//! * **audio thread** — the device output callback owns [`core::EngineCore`].
//!   It mixes the [`graph::Graph`], reads live input from a lock-free ring
//!   filled by the input callback, and pushes captured input to the recorder
//!   and analysis rings. No allocation, locks or I/O.
//! * **streamer thread** — reads clip audio from disk into per-track rings.
//! * **recorder thread** — writes captured input to WAV files.
//! * **control thread(s)** — [`engine::AudioEngine`] sends commands through a
//!   bounded ring and drops objects the audio thread hands back.
//! * **workers** — offline render, waveform peaks, imports, analysis.

pub mod core;
pub mod device;
pub mod engine;
pub mod error;
pub mod graph;
pub mod graph_build;
pub mod import;
pub mod recorder;
pub mod render;
pub mod shared;
pub mod source;
pub mod streamer;
pub mod waveform;
pub mod wav;

pub use engine::{AudioEngine, BackendChoice, EngineStatus, NullOptions, RecordingOutcome};
pub use error::{AudioError, Result};
