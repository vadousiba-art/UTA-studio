//! Comparisons: sung take vs target notes, take vs reference take (DTW), and
//! attempt-to-attempt metric deltas.

use serde::{Deserialize, Serialize};

use crate::notes::{analyze_sustain, segment_notes, DetectedNote};
use crate::pitch::PitchFrame;
use crate::stats::{mean, median, percentile, std_dev};

/// A note the singer was asked to sing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetNote {
    pub midi: f32,
    /// Start time in seconds relative to the start of the recording.
    pub start: f32,
    pub duration: f32,
}

impl TargetNote {
    pub fn end(&self) -> f32 {
        self.start + self.duration
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteResult {
    pub target: TargetNote,
    /// Median sung pitch over the held part of the note window (MIDI),
    /// after octave folding when enabled.
    pub sung_midi: Option<f32>,
    /// Signed pitch error in cents (+ = sharp).
    pub error_cents: Option<f32>,
    /// Octaves removed by folding (e.g. -1 = sung an octave lower).
    pub octave_shift: i32,
    /// Detected start minus target start, ms (+ = late).
    pub onset_error_ms: Option<f32>,
    /// Fraction of the note window that was confidently voiced.
    pub coverage: f32,
    /// Steadiness within the note (cents), if long enough to measure.
    pub wobble_cents: Option<f32>,
    /// False when too little of the note could be measured to grade it.
    pub measurable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetComparison {
    pub notes: Vec<NoteResult>,
    pub measurable_notes: usize,
    pub mean_abs_error_cents: Option<f32>,
    /// Fraction of measurable notes within ±25 cents.
    pub within_25_cents: Option<f32>,
    /// Fraction of measurable notes within ±50 cents.
    pub within_50_cents: Option<f32>,
    pub mean_onset_error_ms: Option<f32>,
    /// Spread of onset errors (std-dev, ms): rhythmic consistency.
    pub onset_spread_ms: Option<f32>,
}

/// Minimum voiced coverage of a note window for it to be graded.
pub const MIN_COVERAGE: f32 = 0.5;

pub fn compare_to_targets(frames: &[PitchFrame], targets: &[TargetNote], octave_tolerant: bool) -> TargetComparison {
    let detected = segment_notes(frames, 0.06, 0.05);
    let mut results = Vec::with_capacity(targets.len());
    for t in targets {
        // skip the attack: first 25% of the note (at most 150 ms)
        let held_start = t.start + (t.duration * 0.25).min(0.15);
        let window: Vec<&PitchFrame> = frames
            .iter()
            .filter(|f| f.time >= held_start && f.time <= t.end())
            .collect();
        let voiced: Vec<f32> = window.iter().filter_map(|f| f.midi()).collect();
        let coverage = if window.is_empty() {
            0.0
        } else {
            voiced.len() as f32 / window.len() as f32
        };
        let measurable = coverage >= MIN_COVERAGE && voiced.len() >= 5;
        let mut octave_shift = 0;
        let sung = median(&voiced).map(|m| {
            if octave_tolerant {
                let shift = ((m - t.midi) / 12.0).round() as i32;
                octave_shift = shift;
                m - 12.0 * shift as f32
            } else {
                m
            }
        });
        let onset = closest_onset(&detected, t);
        let note_frames: Vec<PitchFrame> = frames
            .iter()
            .filter(|f| f.time >= t.start && f.time <= t.end())
            .copied()
            .collect();
        let wobble = analyze_sustain(&note_frames, None).map(|s| s.wobble_cents);
        results.push(NoteResult {
            target: *t,
            sung_midi: if measurable { sung } else { None },
            error_cents: if measurable { sung.map(|s| (s - t.midi) * 100.0) } else { None },
            octave_shift: if measurable { octave_shift } else { 0 },
            onset_error_ms: onset.map(|o| (o - t.start) * 1000.0),
            coverage,
            wobble_cents: wobble,
            measurable,
        });
    }
    let errs: Vec<f32> = results.iter().filter_map(|r| r.error_cents).collect();
    let abs: Vec<f32> = errs.iter().map(|e| e.abs()).collect();
    let onsets: Vec<f32> = results.iter().filter_map(|r| r.onset_error_ms).collect();
    let frac = |lim: f32| {
        if abs.is_empty() {
            None
        } else {
            Some(abs.iter().filter(|e| **e <= lim).count() as f32 / abs.len() as f32)
        }
    };
    TargetComparison {
        measurable_notes: errs.len(),
        mean_abs_error_cents: mean(&abs),
        within_25_cents: frac(25.0),
        within_50_cents: frac(50.0),
        mean_onset_error_ms: mean(&onsets),
        onset_spread_ms: if onsets.len() >= 2 { std_dev(&onsets) } else { None },
        notes: results,
    }
}

fn closest_onset(detected: &[DetectedNote], t: &TargetNote) -> Option<f32> {
    detected
        .iter()
        .map(|n| n.start)
        .filter(|s| (s - t.start).abs() <= 0.35)
        .min_by(|a, b| (a - t.start).abs().total_cmp(&(b - t.start).abs()))
}

/// Result of aligning a take against a reference contour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContourComparison {
    /// Mean |pitch difference| over aligned voiced pairs, cents.
    pub mean_abs_pitch_diff_cents: Option<f32>,
    /// Median signed pitch difference (take − reference), cents.
    pub median_pitch_diff_cents: Option<f32>,
    /// Median time offset of the take relative to the reference, ms (+ = late).
    pub median_timing_offset_ms: f32,
    /// 90th-percentile |timing offset|, ms.
    pub p90_abs_timing_offset_ms: f32,
    /// Fraction of reference voiced frames matched to voiced take frames.
    pub voiced_overlap: f32,
    /// Alignment path as (take_time, reference_time) pairs, decimated.
    pub path: Vec<(f32, f32)>,
}

/// Align two contours with band-constrained dynamic time warping.
///
/// `max_shift_s` bounds how far the alignment may drift (Sakoe–Chiba band).
/// Both contours must use the same hop. Contours are decimated so DTW never
/// processes more than ~2000 points per side.
pub fn compare_contours(take: &[PitchFrame], reference: &[PitchFrame], max_shift_s: f32) -> Option<ContourComparison> {
    if take.len() < 2 || reference.len() < 2 {
        return None;
    }
    let hop = (take[1].time - take[0].time).max(1e-4);
    let step = (take.len().max(reference.len()) / 2000).max(1);
    let a: Vec<Option<f32>> = take.iter().step_by(step).map(|f| f.midi()).collect();
    let b: Vec<Option<f32>> = reference.iter().step_by(step).map(|f| f.midi()).collect();
    let hop = hop * step as f32;
    let (n, m) = (a.len(), b.len());
    let band = ((max_shift_s / hop).ceil() as usize).max(n.abs_diff(m) + 2);

    let cost = |x: Option<f32>, y: Option<f32>| -> f32 {
        match (x, y) {
            (Some(p), Some(q)) => (p - q).abs().min(6.0),
            (None, None) => 0.0,
            _ => 1.5,
        }
    };
    let inf = f32::INFINITY;
    let mut d = vec![inf; n * m];
    let idx = |i: usize, j: usize| i * m + j;
    for i in 0..n {
        let jc = (i as f32 * m as f32 / n as f32) as usize;
        let lo = jc.saturating_sub(band);
        let hi = (jc + band).min(m - 1);
        for j in lo..=hi {
            let c = cost(a[i], b[j]);
            let best = if i == 0 && j == 0 {
                0.0
            } else {
                let mut v = inf;
                if i > 0 {
                    v = v.min(d[idx(i - 1, j)]);
                }
                if j > 0 {
                    v = v.min(d[idx(i, j - 1)]);
                }
                if i > 0 && j > 0 {
                    v = v.min(d[idx(i - 1, j - 1)]);
                }
                v
            };
            d[idx(i, j)] = c + best;
        }
    }
    if !d[idx(n - 1, m - 1)].is_finite() {
        return None;
    }
    // backtrack
    let mut path = Vec::new();
    let (mut i, mut j) = (n - 1, m - 1);
    path.push((i, j));
    while i > 0 || j > 0 {
        let mut cand = Vec::with_capacity(3);
        if i > 0 && j > 0 {
            cand.push((d[idx(i - 1, j - 1)], i - 1, j - 1));
        }
        if i > 0 {
            cand.push((d[idx(i - 1, j)], i - 1, j));
        }
        if j > 0 {
            cand.push((d[idx(i, j - 1)], i, j - 1));
        }
        let (_, ni, nj) = cand
            .into_iter()
            .min_by(|x, y| x.0.total_cmp(&y.0))
            .expect("at least one predecessor");
        i = ni;
        j = nj;
        path.push((i, j));
    }
    path.reverse();

    let mut diffs = Vec::new();
    let mut offsets = Vec::new();
    let mut ref_voiced = 0usize;
    let mut matched = 0usize;
    for &(i, j) in &path {
        offsets.push((i as f32 - j as f32) * hop * 1000.0);
        if b[j].is_some() {
            ref_voiced += 1;
            if let (Some(p), Some(q)) = (a[i], b[j]) {
                matched += 1;
                diffs.push((p - q) * 100.0);
            }
        }
    }
    let abs: Vec<f32> = diffs.iter().map(|x| x.abs()).collect();
    let abs_off: Vec<f32> = offsets.iter().map(|x| x.abs()).collect();
    let dec = (path.len() / 400).max(1);
    Some(ContourComparison {
        mean_abs_pitch_diff_cents: mean(&abs),
        median_pitch_diff_cents: median(&diffs),
        median_timing_offset_ms: median(&offsets).unwrap_or(0.0),
        p90_abs_timing_offset_ms: percentile(&abs_off, 90.0).unwrap_or(0.0),
        voiced_overlap: if ref_voiced == 0 { 0.0 } else { matched as f32 / ref_voiced as f32 },
        path: path
            .iter()
            .step_by(dec)
            .map(|&(i, j)| (i as f32 * hop, j as f32 * hop))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::{track_pitch, PitchConfig};
    use crate::synth;

    fn targets(notes: &[(f32, f32)], gap: f32, lead: f32) -> Vec<TargetNote> {
        let mut t = lead;
        notes
            .iter()
            .map(|&(midi, dur)| {
                let n = TargetNote { midi, start: t, duration: dur };
                t += dur + gap;
                n
            })
            .collect()
    }

    #[test]
    fn accurate_scale_scores_well() {
        let sr = 48_000;
        let notes = [(48.0, 0.5), (50.0, 0.5), (52.0, 0.5)];
        let sig = synth::note_sequence(&notes, 0.1, 0.2, sr);
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let c = compare_to_targets(&frames, &targets(&notes, 0.1, 0.2), false);
        assert_eq!(c.measurable_notes, 3);
        assert!(c.mean_abs_error_cents.unwrap() < 5.0);
        assert_eq!(c.within_25_cents, Some(1.0));
        assert!(c.mean_onset_error_ms.unwrap().abs() < 30.0, "{:?}", c.mean_onset_error_ms);
    }

    #[test]
    fn flat_singing_and_octave_folding() {
        let sr = 48_000;
        // sung 30 cents flat and an octave low
        let sung = [(47.7 - 12.0, 0.5), (49.7 - 12.0, 0.5)];
        let sig = synth::note_sequence(&sung, 0.1, 0.2, sr);
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let t = targets(&[(48.0, 0.5), (50.0, 0.5)], 0.1, 0.2);
        let folded = compare_to_targets(&frames, &t, true);
        for n in &folded.notes {
            assert_eq!(n.octave_shift, -1);
            assert!((n.error_cents.unwrap() + 30.0).abs() < 6.0, "{:?}", n.error_cents);
        }
        let strict = compare_to_targets(&frames, &t, false);
        assert!(strict.notes[0].error_cents.unwrap() < -1100.0);
    }

    #[test]
    fn silence_is_not_graded() {
        let sr = 48_000;
        let sig = vec![0.0f32; sr as usize * 2];
        let frames = track_pitch(&sig, PitchConfig::new(sr));
        let c = compare_to_targets(&frames, &targets(&[(48.0, 0.5)], 0.1, 0.2), false);
        assert_eq!(c.measurable_notes, 0);
        assert!(c.mean_abs_error_cents.is_none());
        assert!(!c.notes[0].measurable);
    }

    #[test]
    fn dtw_finds_delay_and_pitch_offset() {
        let sr = 48_000;
        let notes = [(50.0, 0.4), (53.0, 0.4), (55.0, 0.6)];
        let reference = synth::note_sequence(&notes, 0.05, 0.2, sr);
        // take: 150 ms late and 20 cents sharp
        let sharp: Vec<(f32, f32)> = notes.iter().map(|(m, d)| (m + 0.2, *d)).collect();
        let take = synth::note_sequence(&sharp, 0.05, 0.35, sr);
        let cfg = PitchConfig::new(sr);
        let c = compare_contours(&track_pitch(&take, cfg), &track_pitch(&reference, cfg), 1.0).unwrap();
        assert!((c.median_timing_offset_ms - 150.0).abs() < 30.0, "{c:?}");
        assert!((c.median_pitch_diff_cents.unwrap() - 20.0).abs() < 6.0, "{:?}", c.median_pitch_diff_cents);
        assert!(c.voiced_overlap > 0.8);
    }
}
