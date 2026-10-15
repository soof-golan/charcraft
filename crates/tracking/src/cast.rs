//! Numeric conversions the lints forbid as `as` casts, in one place.

use anim_core::num::{f64_to_f32, round_f64_to_i64, u32_to_usize, usize_to_f64};

/// A count as `f32` (exact below 2^24, which every size here is).
pub const fn f(n: usize) -> f32 {
    f64_to_f32(usize_to_f64(n))
}

/// A `u32` as `f32`.
pub fn fu(n: u32) -> f32 {
    f(u32_to_usize(n))
}

/// Floors an `f32` into `i64`, saturating; `NaN` becomes 0.
pub fn floor_i64(v: f32) -> i64 {
    round_f64_to_i64(f64::from(v).floor())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_saturate() {
        assert_eq!(floor_i64(-0.5), -1);
        assert_eq!(floor_i64(f32::NAN), 0);
        assert_eq!(floor_i64(f32::INFINITY), i64::MAX);
        assert!((f(640) - 640.0).abs() < f32::EPSILON);
        assert!((fu(7) - 7.0).abs() < f32::EPSILON);
    }
}
