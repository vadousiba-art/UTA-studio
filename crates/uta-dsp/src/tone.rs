//! Reference-tone and metronome-click generators (real-time safe).

use std::f32::consts::TAU;

/// MIDI note number to frequency (A4 = 69 = 440 Hz).
#[inline]
pub fn midi_to_hz(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

/// Frequency to (fractional) MIDI note number.
#[inline]
pub fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// Cents difference of `hz` relative to `reference_hz`.
#[inline]
pub fn cents_between(hz: f32, reference_hz: f32) -> f32 {
    1200.0 * (hz / reference_hz).log2()
}

const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// Name of the nearest note, e.g. `A4`.
pub fn note_name(midi: f32) -> String {
    let n = midi.round() as i32;
    let name = NOTE_NAMES[n.rem_euclid(12) as usize];
    format!("{}{}", name, n.div_euclid(12) - 1)
}

/// A soft, organ-like reference tone (fundamental plus two quiet harmonics)
/// with a linear attack/release envelope so notes never click.
#[derive(Debug, Clone)]
pub struct ToneGenerator {
    sample_rate: f32,
    phase: f32,
    freq: f32,
    target_freq: f32,
    amp: f32,
    env: f32,
    env_step: f32,
    remaining: u64,
    gate: bool,
}

impl ToneGenerator {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            freq: 440.0,
            target_freq: 440.0,
            amp: 0.25,
            env: 0.0,
            env_step: 1.0 / (0.015 * sample_rate),
            remaining: 0,
            gate: false,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.env_step = 1.0 / (0.015 * sample_rate);
    }

    /// Start a note of `duration_samples` (0 = until `note_off`).
    pub fn note_on(&mut self, freq: f32, amp: f32, duration_samples: u64) {
        if self.env <= 0.0 {
            self.freq = freq;
            self.phase = 0.0;
        }
        self.target_freq = freq;
        self.amp = amp.clamp(0.0, 1.0);
        self.remaining = if duration_samples == 0 { u64::MAX } else { duration_samples };
        self.gate = true;
    }

    pub fn note_off(&mut self) {
        self.gate = false;
        self.remaining = 0;
    }

    pub fn is_active(&self) -> bool {
        self.gate || self.env > 0.0
    }

    /// Add the tone into a mono buffer.
    pub fn render_add(&mut self, out: &mut [f32]) {
        if !self.is_active() {
            return;
        }
        for s in out.iter_mut() {
            if self.gate {
                if self.remaining == 0 {
                    self.gate = false;
                } else {
                    self.remaining -= 1;
                }
            }
            if self.gate {
                self.env = (self.env + self.env_step).min(1.0);
            } else {
                self.env = (self.env - self.env_step).max(0.0);
            }
            // glide 5 ms between notes to avoid discontinuities
            self.freq += (self.target_freq - self.freq) * 0.002;
            self.phase += self.freq / self.sample_rate;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            let p = self.phase * TAU;
            let v = p.sin() + 0.18 * (2.0 * p).sin() + 0.06 * (3.0 * p).sin();
            *s += v * self.amp * self.env * 0.8;
        }
    }
}

/// Metronome click: short decaying sine burst, accented on the downbeat.
#[derive(Debug, Clone)]
pub struct Click {
    sample_rate: f32,
    phase: f32,
    freq: f32,
    env: f32,
    decay: f32,
}

impl Click {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            freq: 1000.0,
            env: 0.0,
            decay: (-1.0 / (0.012 * sample_rate)).exp(),
        }
    }

    pub fn trigger(&mut self, accent: bool, amp: f32) {
        self.freq = if accent { 1600.0 } else { 1000.0 };
        self.env = amp;
        self.phase = 0.0;
    }

    pub fn render_add(&mut self, out: &mut [f32]) {
        if self.env < 1e-4 {
            self.env = 0.0;
            return;
        }
        for s in out.iter_mut() {
            self.phase += self.freq / self.sample_rate;
            *s += (self.phase * TAU).sin() * self.env;
            self.env *= self.decay;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert!((midi_to_hz(69.0) - 440.0).abs() < 1e-3);
        assert!((midi_to_hz(60.0) - 261.6256).abs() < 1e-2);
        assert!((hz_to_midi(220.0) - 57.0).abs() < 1e-4);
        assert!((cents_between(445.0, 440.0) - 19.56).abs() < 0.05);
        assert_eq!(note_name(60.0), "C4");
        assert_eq!(note_name(57.2), "A3");
        assert_eq!(note_name(70.0), "A#4");
    }

    #[test]
    fn tone_has_expected_period_and_ends() {
        let sr = 48_000.0;
        let mut t = ToneGenerator::new(sr);
        t.note_on(200.0, 0.5, 9600);
        let mut buf = vec![0.0f32; 12_000];
        t.render_add(&mut buf);
        // count positive-going zero crossings in the steady part (0.05..0.15 s)
        let seg = &buf[2400..7200];
        let crossings = seg.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((crossings as i32 - 20).abs() <= 1, "{crossings}");
        assert!(!t.is_active());
        assert!(buf[11_000..].iter().all(|s| *s == 0.0));
    }
}
