//! Level conversions.

/// Smallest level reported by meters, in dBFS. Anything quieter is "silence".
pub const MIN_DB: f32 = -120.0;

/// Convert decibels to a linear gain factor.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    if db <= MIN_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Convert a linear gain factor (or absolute amplitude) to decibels.
/// Returns [`MIN_DB`] for zero / denormal values.
#[inline]
pub fn gain_to_db(gain: f32) -> f32 {
    let g = gain.abs();
    if g <= 1e-6 {
        MIN_DB
    } else {
        (20.0 * g.log10()).max(MIN_DB)
    }
}

/// Flush values that would become denormals in recursive filters.
#[inline]
pub fn flush_denormal(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for db in [-60.0f32, -12.0, -6.0, 0.0, 6.0] {
            let g = db_to_gain(db);
            assert!((gain_to_db(g) - db).abs() < 1e-3, "{db}");
        }
        assert!((db_to_gain(-6.0206) - 0.5).abs() < 1e-4);
        assert_eq!(db_to_gain(MIN_DB), 0.0);
        assert_eq!(gain_to_db(0.0), MIN_DB);
    }
}
