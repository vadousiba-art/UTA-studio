//! Small statistics helpers.

pub fn mean(v: &[f32]) -> Option<f32> {
    if v.is_empty() {
        None
    } else {
        Some(v.iter().sum::<f32>() / v.len() as f32)
    }
}

pub fn std_dev(v: &[f32]) -> Option<f32> {
    let m = mean(v)?;
    if v.len() < 2 {
        return Some(0.0);
    }
    let var = v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / (v.len() - 1) as f32;
    Some(var.sqrt())
}

/// Percentile with linear interpolation, `p` in `[0, 100]`.
pub fn percentile(v: &[f32], p: f32) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    let mut s: Vec<f32> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return None;
    }
    s.sort_by(|a, b| a.total_cmp(b));
    let rank = (p.clamp(0.0, 100.0) / 100.0) * (s.len() - 1) as f32;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f32;
    Some(s[lo] * (1.0 - frac) + s[hi] * frac)
}

pub fn median(v: &[f32]) -> Option<f32> {
    percentile(v, 50.0)
}

/// Least-squares slope of `y` over `x`. `None` if fewer than two points or
/// degenerate x.
pub fn linear_slope(x: &[f32], y: &[f32]) -> Option<f32> {
    let n = x.len().min(y.len());
    if n < 2 {
        return None;
    }
    let mx = x[..n].iter().sum::<f32>() / n as f32;
    let my = y[..n].iter().sum::<f32>() / n as f32;
    let mut num = 0.0;
    let mut den = 0.0;
    for i in 0..n {
        num += (x[i] - mx) * (y[i] - my);
        den += (x[i] - mx) * (x[i] - mx);
    }
    if den.abs() < 1e-12 {
        None
    } else {
        Some(num / den)
    }
}

/// Running median filter of odd width; `None` entries are skipped and kept.
pub fn median_filter_opt(v: &[Option<f32>], width: usize) -> Vec<Option<f32>> {
    let half = width / 2;
    let mut out = Vec::with_capacity(v.len());
    let mut window = Vec::with_capacity(width);
    for i in 0..v.len() {
        if v[i].is_none() {
            out.push(None);
            continue;
        }
        window.clear();
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(v.len());
        window.extend(v[lo..hi].iter().flatten().copied());
        out.push(median(&window));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        assert_eq!(mean(&[1.0, 2.0, 3.0]), Some(2.0));
        assert!((std_dev(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]).unwrap() - 2.138).abs() < 1e-3);
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(percentile(&[0.0, 10.0], 25.0), Some(2.5));
        assert!((linear_slope(&[0.0, 1.0, 2.0], &[1.0, 3.0, 5.0]).unwrap() - 2.0).abs() < 1e-6);
        assert_eq!(linear_slope(&[1.0], &[1.0]), None);
        let f = median_filter_opt(&[Some(1.0), Some(100.0), Some(1.0), None, Some(2.0)], 3);
        assert_eq!(f, vec![Some(50.5), Some(1.0), Some(50.5), None, Some(2.0)]);
    }
}
