//! Real-time-safe DSP building blocks for UTA Vocal Lab.
//!
//! Everything in this crate that is called from an audio callback follows the
//! same rules:
//!
//! * no heap allocation after [`Effect::prepare`] has returned,
//! * no locks, no I/O, no panics on valid input,
//! * processing is done on non-interleaved stereo `f32` buffers.
//!
//! Effects expose their parameters through static [`ParamInfo`] tables so the
//! UI, the project format and the audio thread all agree on ids, ranges and
//! defaults without any string handling on the audio thread.

pub mod biquad;
pub mod compressor;
pub mod delay;
pub mod effect;
pub mod eq;
pub mod gate;
pub mod level;
pub mod meter;
pub mod pan;
pub mod reverb;
pub mod smoothing;
pub mod stretch;
pub mod tone;
pub mod utility;

pub use effect::{create_builtin, BuiltinKind, Effect, ParamInfo, ParamScale};
pub use level::{db_to_gain, gain_to_db};
pub use meter::{MeterReading, PeakRmsMeter};
pub use pan::constant_power_pan;
