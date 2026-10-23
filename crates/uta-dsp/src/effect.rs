//! The [`Effect`] trait and parameter descriptors shared by all built-in
//! effects.

use serde::{Deserialize, Serialize};

/// How a parameter should be presented / mapped by a UI.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ParamScale {
    Linear,
    /// Logarithmic mapping (frequencies, times). `min` must be > 0.
    Log,
    /// 0 = off, 1 = on.
    Toggle,
    /// Discrete choice; the value is the index into the labels.
    Choice { labels: &'static [&'static str] },
}

/// Static description of one effect parameter.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamInfo {
    /// Stable identifier used in project files and presets. Never rename.
    pub id: &'static str,
    pub name: &'static str,
    pub unit: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub scale: ParamScale,
}

impl ParamInfo {
    pub fn clamp(&self, v: f32) -> f32 {
        if !v.is_finite() {
            return self.default;
        }
        let v = v.clamp(self.min, self.max);
        match self.scale {
            ParamScale::Toggle | ParamScale::Choice { .. } => v.round(),
            _ => v,
        }
    }
}

/// `const` constructor used to build the static parameter tables.
pub const fn param(
    id: &'static str,
    name: &'static str,
    unit: &'static str,
    min: f32,
    max: f32,
    default: f32,
    scale: ParamScale,
) -> ParamInfo {
    ParamInfo {
        id,
        name,
        unit,
        min,
        max,
        default,
        scale,
    }
}

/// Look up a parameter index by id.
pub fn param_index(params: &[ParamInfo], id: &str) -> Option<usize> {
    params.iter().position(|p| p.id == id)
}

/// A stereo audio effect that is safe to run on the real-time thread once
/// prepared.
pub trait Effect: Send {
    fn kind(&self) -> BuiltinKind;

    fn params(&self) -> &'static [ParamInfo] {
        self.kind().params()
    }

    /// Allocate all buffers for the given sample rate and maximum block size.
    /// Called off the audio thread.
    fn prepare(&mut self, sample_rate: f32, max_block: usize);

    /// Clear internal state (delay lines, envelopes) without reallocating.
    fn reset(&mut self);

    /// Set a parameter by index. Values are clamped to the declared range.
    fn set_param(&mut self, index: usize, value: f32);

    fn get_param(&self, index: usize) -> f32;

    /// Process a block in place. Both slices have the same length, which is
    /// never larger than the `max_block` given to `prepare`.
    fn process(&mut self, left: &mut [f32], right: &mut [f32]);

    /// Latency introduced by the effect, in samples.
    fn latency_samples(&self) -> usize {
        0
    }

    /// Current gain reduction in dB (compressor / gate), for metering.
    fn gain_reduction_db(&self) -> f32 {
        0.0
    }

    /// Convenience: set by parameter id. Returns false for unknown ids.
    fn set_param_by_id(&mut self, id: &str, value: f32) -> bool {
        match param_index(self.params(), id) {
            Some(i) => {
                self.set_param(i, value);
                true
            }
            None => false,
        }
    }

    /// All current parameter values as `(id, value)` pairs.
    fn param_values(&self) -> Vec<(&'static str, f32)> {
        self.params()
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id, self.get_param(i)))
            .collect()
    }
}

/// The built-in effect types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BuiltinKind {
    Eq,
    Compressor,
    Gate,
    Reverb,
    Delay,
    Utility,
}

impl BuiltinKind {
    pub const ALL: [BuiltinKind; 6] = [
        BuiltinKind::Eq,
        BuiltinKind::Compressor,
        BuiltinKind::Gate,
        BuiltinKind::Reverb,
        BuiltinKind::Delay,
        BuiltinKind::Utility,
    ];

    pub fn id(self) -> &'static str {
        match self {
            BuiltinKind::Eq => "eq",
            BuiltinKind::Compressor => "compressor",
            BuiltinKind::Gate => "gate",
            BuiltinKind::Reverb => "reverb",
            BuiltinKind::Delay => "delay",
            BuiltinKind::Utility => "utility",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            BuiltinKind::Eq => "UTA EQ",
            BuiltinKind::Compressor => "UTA Compressor",
            BuiltinKind::Gate => "UTA Gate",
            BuiltinKind::Reverb => "UTA Reverb",
            BuiltinKind::Delay => "UTA Delay",
            BuiltinKind::Utility => "UTA Utility",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }

    pub fn params(self) -> &'static [ParamInfo] {
        match self {
            BuiltinKind::Eq => crate::eq::PARAMS,
            BuiltinKind::Compressor => crate::compressor::PARAMS,
            BuiltinKind::Gate => crate::gate::PARAMS,
            BuiltinKind::Reverb => crate::reverb::PARAMS,
            BuiltinKind::Delay => crate::delay::PARAMS,
            BuiltinKind::Utility => crate::utility::PARAMS,
        }
    }
}

/// Instantiate a built-in effect with default parameters. The caller must
/// call [`Effect::prepare`] before processing.
pub fn create_builtin(kind: BuiltinKind) -> Box<dyn Effect> {
    match kind {
        BuiltinKind::Eq => Box::new(crate::eq::Equalizer::new()),
        BuiltinKind::Compressor => Box::new(crate::compressor::Compressor::new()),
        BuiltinKind::Gate => Box::new(crate::gate::Gate::new()),
        BuiltinKind::Reverb => Box::new(crate::reverb::Reverb::new()),
        BuiltinKind::Delay => Box::new(crate::delay::Delay::new()),
        BuiltinKind::Utility => Box::new(crate::utility::Utility::new()),
    }
}

/// Helper for effects that keep their parameter values in a fixed array.
#[derive(Debug, Clone)]
pub struct ParamValues<const N: usize> {
    pub values: [f32; N],
    infos: &'static [ParamInfo],
}

impl<const N: usize> ParamValues<N> {
    pub fn new(infos: &'static [ParamInfo]) -> Self {
        assert_eq!(infos.len(), N, "param table length mismatch");
        let mut values = [0.0; N];
        for (v, p) in values.iter_mut().zip(infos) {
            *v = p.default;
        }
        Self { values, infos }
    }

    /// Store a clamped value; returns true if it changed.
    pub fn set(&mut self, index: usize, value: f32) -> bool {
        if index >= N {
            return false;
        }
        let v = self.infos[index].clamp(value);
        let changed = self.values[index] != v;
        self.values[index] = v;
        changed
    }

    pub fn get(&self, index: usize) -> f32 {
        self.values.get(index).copied().unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_has_valid_param_table() {
        for kind in BuiltinKind::ALL {
            let params = kind.params();
            assert!(!params.is_empty());
            for p in params {
                assert!(p.min <= p.default && p.default <= p.max, "{} {}", kind.id(), p.id);
                if p.scale == ParamScale::Log {
                    assert!(p.min > 0.0, "log scale needs positive min: {}", p.id);
                }
            }
            // ids unique
            for (i, a) in params.iter().enumerate() {
                for b in &params[i + 1..] {
                    assert_ne!(a.id, b.id);
                }
            }
            assert_eq!(BuiltinKind::from_id(kind.id()), Some(kind));
        }
    }

    #[test]
    fn every_builtin_processes_silence_to_silence_and_stays_finite() {
        for kind in BuiltinKind::ALL {
            let mut fx = create_builtin(kind);
            fx.prepare(48_000.0, 512);
            let mut l = vec![0.0f32; 512];
            let mut r = vec![0.0f32; 512];
            fx.process(&mut l, &mut r);
            assert!(l.iter().chain(&r).all(|s| s.abs() < 1e-6), "{}", kind.id());
            // noise burst must stay finite
            let mut seed = 1u32;
            for (a, b) in l.iter_mut().zip(r.iter_mut()) {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                *a = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                *b = -*a;
            }
            for _ in 0..50 {
                fx.process(&mut l, &mut r);
            }
            assert!(l.iter().chain(&r).all(|s| s.is_finite()), "{}", kind.id());
        }
    }

    #[test]
    fn set_param_clamps() {
        let mut fx = create_builtin(BuiltinKind::Utility);
        assert!(fx.set_param_by_id("gain", 1000.0));
        let idx = param_index(fx.params(), "gain").unwrap();
        assert_eq!(fx.get_param(idx), fx.params()[idx].max);
        assert!(!fx.set_param_by_id("nope", 1.0));
        fx.set_param(idx, f32::NAN);
        assert_eq!(fx.get_param(idx), fx.params()[idx].default);
    }
}
