//! Parameter smoothing to avoid zipper noise when values change.

/// One-pole exponential smoother. Reaches ~63% of a step after `time_ms`.
#[derive(Debug, Clone, Copy)]
pub struct Smoother {
    current: f32,
    target: f32,
    coeff: f32,
}

impl Smoother {
    pub fn new(value: f32) -> Self {
        Self {
            current: value,
            target: value,
            coeff: 0.0,
        }
    }

    /// Configure the smoothing time for a given sample rate.
    pub fn set_time(&mut self, sample_rate: f32, time_ms: f32) {
        let samples = (time_ms * 0.001 * sample_rate).max(1.0);
        self.coeff = (-1.0 / samples).exp();
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    /// Jump straight to a value (used on reset / transport start).
    pub fn snap(&mut self, value: f32) {
        self.current = value;
        self.target = value;
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn is_settled(&self) -> bool {
        (self.current - self.target).abs() < 1e-5
    }

    /// Advance one sample and return the smoothed value.
    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f32 {
        if self.current != self.target {
            let next = self.target + (self.current - self.target) * self.coeff;
            // f32 rounding can stall the approach a few ulps short of the
            // target; snap once progress stops or the error is inaudible.
            self.current = if next == self.current || (next - self.target).abs() < 1e-6 {
                self.target
            } else {
                next
            };
        }
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converges_to_target() {
        let mut s = Smoother::new(0.0);
        s.set_time(48_000.0, 10.0);
        s.set_target(1.0);
        let mut last = 0.0;
        for _ in 0..480 {
            let v = s.next();
            assert!(v >= last, "monotonic");
            last = v;
        }
        // after one time constant ~63%
        assert!((last - 0.632).abs() < 0.02, "{last}");
        for _ in 0..48_000 {
            s.next();
        }
        assert!(s.is_settled());
    }
}
