//! Panning.

/// Constant-power (−3 dB centre) pan law.
///
/// `pan` is in `[-1, 1]` (left .. right). Returns `(left_gain, right_gain)`.
#[inline]
pub fn constant_power_pan(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    let angle = (p + 1.0) * std::f32::consts::FRAC_PI_4; // 0 .. pi/2
    (angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn law() {
        let (l, r) = constant_power_pan(0.0);
        assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((r - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        let (l, r) = constant_power_pan(-1.0);
        assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6);
        let (l, r) = constant_power_pan(1.0);
        assert!(l.abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        // power is constant
        for p in [-0.7f32, -0.2, 0.3, 0.9] {
            let (l, r) = constant_power_pan(p);
            assert!((l * l + r * r - 1.0).abs() < 1e-5);
        }
    }
}
