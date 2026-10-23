//! Offline pitch-preserving time stretching (WSOLA).
//!
//! Used by the Song Lab to render slowed-down practice copies of a backing
//! track in a background worker. Not real-time: it allocates and runs over a
//! whole buffer. Quality is good for moderate rates (0.5x – 1.25x) on music;
//! extreme rates will smear transients.

/// Time-stretch a set of channels (all the same length) by `speed`
/// (0.75 = 75% speed / longer output). Alignment is computed on the channel
/// average and applied identically to every channel so stereo image is kept.
///
/// `progress` is called with values in `[0, 1]`; return `false` to cancel,
/// in which case `None` is returned.
pub fn time_stretch(
    channels: &[Vec<f32>],
    sample_rate: u32,
    speed: f32,
    mut progress: impl FnMut(f32) -> bool,
) -> Option<Vec<Vec<f32>>> {
    let speed = speed.clamp(0.25, 4.0);
    let n_in = channels.first().map(|c| c.len()).unwrap_or(0);
    if n_in == 0 || (speed - 1.0).abs() < 1e-4 {
        return Some(channels.to_vec());
    }
    // Frame of ~40 ms, 50% overlap Hann.
    let frame = ((sample_rate as f32 * 0.04) as usize).next_power_of_two().max(256);
    let hop_out = frame / 2;
    let hop_in = hop_out as f32 * speed;
    let tolerance = frame / 4;
    let window: Vec<f32> = (0..frame)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / frame as f32).cos())
        .collect();

    let mono: Vec<f32> = (0..n_in)
        .map(|i| channels.iter().map(|c| c[i]).sum::<f32>() / channels.len() as f32)
        .collect();

    let n_out = (n_in as f32 / speed).ceil() as usize;
    let mut out: Vec<Vec<f32>> = vec![vec![0.0; n_out + frame]; channels.len()];
    let mut norm = vec![0.0f32; n_out + frame];

    let get = |sig: &[f32], i: isize| -> f32 {
        if i < 0 || i as usize >= sig.len() {
            0.0
        } else {
            sig[i as usize]
        }
    };

    let mut prev_src: isize = 0; // source position of the previously copied frame
    let mut out_pos = 0usize;
    let total_frames = n_out / hop_out + 1;
    let mut frame_idx = 0usize;
    while out_pos < n_out {
        let nominal = (frame_idx as f32 * hop_in) as isize;
        let src = if frame_idx == 0 {
            0
        } else {
            // The natural continuation of the previous frame.
            let natural = prev_src + hop_out as isize;
            let cmp_len = hop_out;
            let score = |cand: isize| -> f32 {
                let mut acc = 0.0f32;
                let mut i = 0;
                while i < cmp_len {
                    acc += get(&mono, natural + i as isize) * get(&mono, cand + i as isize);
                    i += 4;
                }
                acc
            };
            // coarse search
            let mut best = nominal;
            let mut best_score = f32::MIN;
            let t = tolerance as isize;
            let mut k = -t;
            while k <= t {
                let s = score(nominal + k);
                if s > best_score {
                    best_score = s;
                    best = nominal + k;
                }
                k += 8;
            }
            // refine
            let coarse = best;
            for k in -8..=8 {
                let s = score(coarse + k);
                if s > best_score {
                    best_score = s;
                    best = coarse + k;
                }
            }
            best.max(0)
        };
        for (ch_out, ch_in) in out.iter_mut().zip(channels) {
            for i in 0..frame {
                ch_out[out_pos + i] += get(ch_in, src + i as isize) * window[i];
            }
        }
        for i in 0..frame {
            norm[out_pos + i] += window[i];
        }
        prev_src = src;
        out_pos += hop_out;
        frame_idx += 1;
        if frame_idx % 64 == 0 && !progress(frame_idx as f32 / total_frames as f32) {
            return None;
        }
    }
    for ch in out.iter_mut() {
        for (s, n) in ch.iter_mut().zip(&norm) {
            if *n > 1e-3 {
                *s /= n;
            }
        }
        ch.truncate(n_out);
    }
    progress(1.0);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_crossing_freq(sig: &[f32], sr: f32) -> f32 {
        let c = sig.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        c as f32 * sr / sig.len() as f32
    }

    #[test]
    fn half_speed_doubles_length_and_keeps_pitch() {
        let sr = 48_000u32;
        let sig: Vec<f32> = (0..sr as usize)
            .map(|i| 0.5 * (std::f32::consts::TAU * 330.0 * i as f32 / sr as f32).sin())
            .collect();
        let out = time_stretch(&[sig.clone(), sig], sr, 0.5, |_| true).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].len(), 2 * sr as usize);
        let f = zero_crossing_freq(&out[0][10_000..80_000], sr as f32);
        assert!((f - 330.0).abs() < 4.0, "{f}");
        // amplitude preserved roughly
        let peak = out[0][10_000..80_000].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.08, "{peak}");
    }

    #[test]
    fn cancellation() {
        let sig = vec![0.1f32; 480_000];
        assert!(time_stretch(&[sig], 48_000, 0.8, |_| false).is_none());
    }
}
