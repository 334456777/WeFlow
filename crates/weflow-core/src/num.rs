//! Numeric conversions that saturate instead of truncating.
//!
//! JSON numbers from the database layer and WeFlow's desktop app's settings arrive as `f64` and are rounded the way
//! WeFlow's desktop app's `Math.floor` / `Math.round` / `Math.trunc` round them. Rust's float-to-integer `as` already
//! saturates at the target's bounds and maps NaN to 0, which is the behavior wanted here, so it is used in one
//! place instead of at every call site.
//!
//! The database layer takes limits, offsets and Unix seconds as `i32`; values from requests are wider, and an
//! integer `as` would wrap them, so they saturate instead.
//!
//! Ratios, means and scales take counts and timestamps as `f64`, which holds every integer up to 2^53 exactly: far
//! above any count, and a Unix time in milliseconds is about 2^41.

/// `v` as an `i32`, saturating at `i32::MIN` / `i32::MAX` when it is out of range.
#[must_use]
pub fn saturating_i32<T>(v: T) -> i32
where
    T: Copy + Default + PartialOrd + TryInto<i32>,
{
    v.try_into()
        .unwrap_or(if v < T::default() { i32::MIN } else { i32::MAX })
}

/// `n`, a count or a length, as the `i64` the database layer and the contract use: saturating at `i64::MAX`, which
/// no count reaches.
#[must_use]
pub fn count_i64(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// `n`, a count or a length, as an `f64` to divide or scale by: exact, as no count reaches 2^53.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "a count is below 2^53, where an `f64` holds every integer"
)]
pub fn count_f64(n: usize) -> f64 {
    n as f64
}

/// `n`, a count, a sum or a timestamp the database layer gives as `i64`, as an `f64`: exact within ±2^53, the
/// nearest `f64` beyond.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "counts, sums and timestamps are within ±2^53, where an `f64` holds every integer"
)]
pub fn i64_f64(n: i64) -> f64 {
    n as f64
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
    fn counts_saturate_into_i64() {
        assert_eq!(count_i64(0), 0);
        assert_eq!(count_i64(1 << 20), 1 << 20);
        assert_eq!(count_i64(usize::MAX), i64::MAX);
    }

    #[test]
    fn integers_below_2_pow_53_are_exact_as_f64() {
        assert_eq!(count_f64(0), 0.0);
        assert_eq!(count_f64(1 << 53), 9_007_199_254_740_992.0);
        assert_eq!(i64_f64(-(1 << 53)), -9_007_199_254_740_992.0);
        assert_eq!(i64_f64(1_700_000_000_123), 1_700_000_000_123.0);
    }

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
