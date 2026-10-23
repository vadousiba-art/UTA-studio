//! Note segmentation and sustained-note analysis.

use serde::{Deserialize, Serialize};

use crate::pitch::PitchFrame;
use crate::stats::{linear_slope, mean, median, std_dev};
use crate::vibrato::{detect_vibrato, Vibrato};

/// A stable-pitch region found in a contour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedNote {
    pub start: f32,
    pub end: f32,
    /// Median pitch (fractional MIDI).
    pub midi: f32,
    /// Standard deviation of the pitch inside the note, cents.
    pub spread_cents: f32,
    pub mean_confidence: f32,
}

impl DetectedNote {
    pub fn duration(&self) -> f32 {
        self.end - self.start
    }
}

/// Split a contour into notes. A new note starts after an unvoiced gap of
/// more than `max_gap` seconds, or when the pitch stays more than 80 cents
/// away from the current note for at least 30 ms. Notes shorter than
/// `min_duration` are dropped (they are usually glides or scoops).
pub fn segment_notes(frames: &[PitchFrame], min_duration: f32, max_gap: f32) -> Vec<DetectedNote> {
    let hop = if frames.len() > 1 {
        (frames[1].time - frames[0].time).max(1e-4)
    } else {
        0.01
    };
    let away_frames = ((0.03 / hop).ceil() as usize).max(2);
    let mut notes = Vec::new();
    let mut cur: Vec<(f32, f32, f32)> = Vec::new(); // (time, midi, confidence)
    let mut last_voiced_time = f32::MIN;
    let mut away: Vec<(f32, f32, f32)> = Vec::new();

    let flush = |cur: &mut Vec<(f32, f32, f32)>, notes: &mut Vec<DetectedNote>| {
        if cur.len() >= 2 {
            let start = cur[0].0 - hop / 2.0;
            let end = cur[cur.len() - 1].0 + hop / 2.0;
            if end - start >= min_duration {
                let midis: Vec<f32> = cur.iter().map(|c| c.1).collect();
                let confs: Vec<f32> = cur.iter().map(|c| c.2).collect();
                let med = median(&midis).unwrap_or(0.0);
                let cents: Vec<f32> = midis.iter().map(|m| (m - med) * 100.0).collect();
                notes.push(DetectedNote {
                    start,
                    end,
                    midi: med,
                    spread_cents: std_dev(&cents).unwrap_or(0.0),
                    mean_confidence: mean(&confs).unwrap_or(0.0),
                });
            }
        }
        cur.clear();
    };

    for f in frames {
        let Some(m) = f.midi() else { continue };
        if f.time - last_voiced_time > max_gap + hop * 0.5 {
            cur.append(&mut away);
            flush(&mut cur, &mut notes);
        }
        last_voiced_time = f.time;
        if cur.is_empty() {
            cur.push((f.time, m, f.confidence));
            continue;
        }
        // reference: median of the most recent (up to 10) frames of the note
        let tail = &cur[cur.len().saturating_sub(10)..];
        let ref_m = median(&tail.iter().map(|c| c.1).collect::<Vec<_>>()).unwrap_or(m);
        if (m - ref_m).abs() > 0.8 {
            away.push((f.time, m, f.confidence));
            if away.len() >= away_frames {
                flush(&mut cur, &mut notes);
                cur.append(&mut away);
            }
        } else {
            cur.append(&mut away);
            cur.push((f.time, m, f.confidence));
        }
    }
    cur.append(&mut away);
    flush(&mut cur, &mut notes);
    notes
}

/// Measurements of how a sustained note was held.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SustainAnalysis {
    pub start: f32,
    pub duration: f32,
    /// Median pitch of the held part (MIDI).
    pub midi: f32,
    /// Error of the median pitch vs the target, in cents (if a target given).
    pub target_error_cents: Option<f32>,
    /// Linear pitch drift across the note, cents per second.
    pub drift_cents_per_second: f32,
    /// Total drift across the note, cents.
    pub drift_cents: f32,
    /// Unsteadiness: std-dev of the pitch around its trend after averaging
    /// out vibrato-rate movement, in cents. Lower = steadier.
    pub wobble_cents: f32,
    pub vibrato: Option<Vibrato>,
}

/// Analyse the frames of a single held note. The first 120 ms (onset) are
/// skipped so scoops into the note are not counted as drift.
pub fn analyze_sustain(frames: &[PitchFrame], target_midi: Option<f32>) -> Option<SustainAnalysis> {
    let voiced: Vec<&PitchFrame> = frames.iter().filter(|f| f.is_voiced()).collect();
    if voiced.len() < 20 {
        return None;
    }
    let start = voiced[0].time;
    let hop = (voiced[1].time - voiced[0].time).max(1e-4);
    let held: Vec<&PitchFrame> = voiced.iter().copied().filter(|f| f.time >= start + 0.12).collect();
    if held.len() < 15 {
        return None;
    }
    let times: Vec<f32> = held.iter().map(|f| f.time).collect();
    let midis: Vec<f32> = held.iter().filter_map(|f| f.midi()).collect();
    let med = median(&midis)?;
    let cents: Vec<f32> = midis.iter().map(|m| (m - med) * 100.0).collect();
    let slope = linear_slope(&times, &cents).unwrap_or(0.0);
    let duration = times[times.len() - 1] - times[0] + hop;

    // Smooth with a 200 ms moving average (one cycle of 5 Hz vibrato) and
    // measure what remains around the linear trend.
    let w = ((0.2 / hop).round() as usize).max(1);
    let mut smooth = Vec::with_capacity(cents.len());
    for i in 0..cents.len() {
        let lo = i.saturating_sub(w / 2);
        let hi = (i + w / 2 + 1).min(cents.len());
        smooth.push(cents[lo..hi].iter().sum::<f32>() / (hi - lo) as f32);
    }
    let t0 = times[0];
    let resid: Vec<f32> = smooth
        .iter()
        .zip(&times)
        .map(|(c, t)| c - slope * (t - t0))
        .collect();
    let resid_mean = mean(&resid).unwrap_or(0.0);
    let resid: Vec<f32> = resid.iter().map(|r| r - resid_mean).collect();

    Some(SustainAnalysis {
        start,
        duration,
        midi: med,
        target_error_cents: target_midi.map(|t| (med - t) * 100.0),
        drift_cents_per_second: slope,
        drift_cents: slope * duration,
        wobble_cents: std_dev(&resid).unwrap_or(0.0),
        vibrato: detect_vibrato(&cents, hop),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::{track_pitch, PitchConfig};
    use crate::synth;

    #[test]
    fn segments_a_scale() {
        let sr = 48_000;
        let notes_in = [(48.0, 0.4), (50.0, 0.4), (52.0, 0.4), (53.0, 0.4), (55.0, 0.6)];
        let sig = synth::note_sequence(&notes_in, 0.1, 0.3, sr);
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let notes = segment_notes(&frames, 0.1, 0.05);
        assert_eq!(notes.len(), 5, "{notes:#?}");
        for (n, (midi, dur)) in notes.iter().zip(notes_in) {
            assert!((n.midi - midi).abs() < 0.1, "{} vs {}", n.midi, midi);
            assert!((n.duration() - dur).abs() < 0.08, "{} vs {}", n.duration(), dur);
        }
        // first note starts after the 0.3 s lead-in (±30 ms)
        assert!((notes[0].start - 0.3).abs() < 0.03, "{}", notes[0].start);
    }

    #[test]
    fn legato_steps_split_without_gaps() {
        let sr = 48_000;
        let mut sig = synth::harmonic_tone(220.0, &[1.0, 0.4], 0.5, sr);
        sig.extend(synth::harmonic_tone(246.94, &[1.0, 0.4], 0.5, sr));
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let notes = segment_notes(&frames, 0.1, 0.05);
        assert_eq!(notes.len(), 2, "{notes:#?}");
    }

    #[test]
    fn sustain_drift_is_measured() {
        let sr = 48_000;
        let sig = synth::drifting_tone(196.0, -40.0, 3.0, sr); // flat by 40 cents over 3 s
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let s = analyze_sustain(&frames, Some(uta_dsp::tone::hz_to_midi(196.0))).unwrap();
        assert!((s.drift_cents_per_second + 13.3).abs() < 2.0, "{s:?}");
        assert!(s.wobble_cents < 3.0, "{s:?}");
        assert!(s.vibrato.is_none());
        // median is around -20 cents (halfway)
        assert!((s.target_error_cents.unwrap() + 21.0).abs() < 4.0, "{s:?}");
    }

    #[test]
    fn steady_note_has_low_wobble() {
        let sr = 48_000;
        let sig = synth::harmonic_tone(261.63, &[1.0, 0.5, 0.2], 2.0, sr);
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let s = analyze_sustain(&frames, Some(60.0)).unwrap();
        assert!(s.target_error_cents.unwrap().abs() < 3.0);
        assert!(s.drift_cents.abs() < 3.0);
        assert!(s.wobble_cents < 1.0);
    }
}
