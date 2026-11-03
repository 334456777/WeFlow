//! Numeric conversions that saturate instead of truncating.
//!
//! JSON numbers from the database layer and WeFlow's desktop app's settings arrive as `f64` and are rounded the way
//! WeFlow's desktop app's `Math.floor` / `Math.round` / `Math.trunc` round them. Rust's float-to-integer `as` already
//! saturates at the target's bounds and maps NaN to 0, which is the behavior wanted here, so it is used in one
//! place instead of at every call site.
//!
//! The database layer takes limits, offsets and Unix seconds as `i32`; values from requests are wider, and an
//! integer `as` would wrap them, so they saturate instead.

/// `v` as an `i32`, saturating at `i32::MIN` / `i32::MAX` when it is out of range.
#[must_use]
pub fn saturating_i32<T>(v: T) -> i32
where
    T: Copy + Default + PartialOrd + TryInto<i32>,
{
    v.try_into()
        .unwrap_or(if v < T::default() { i32::MIN } else { i32::MAX })
}

/// `f` truncated toward zero, saturating at the `i64` bounds; NaN becomes 0.
///
/// Round first (`f.floor()`, `f.round()`) when that is the rounding wanted.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "float-to-integer `as` saturates, which is the conversion wanted"
)]
pub fn saturating_i64(f: f64) -> i64 {
    f as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_saturate_into_i32() {
        assert_eq!(saturating_i32(5_i64), 5);
        assert_eq!(saturating_i32(9_999_999_999_i64), i32::MAX);
        assert_eq!(saturating_i32(i64::MIN), i32::MIN);
        assert_eq!(saturating_i32(usize::MAX), i32::MAX);
    }

    #[test]
    fn floats_saturate() {
        assert_eq!(saturating_i64(1.9), 1);
        assert_eq!(saturating_i64(-1.9), -1);
        assert_eq!(saturating_i64(f64::NAN), 0);
        assert_eq!(saturating_i64(1e300), i64::MAX);
        assert_eq!(saturating_i64(f64::NEG_INFINITY), i64::MIN);
    }
}
