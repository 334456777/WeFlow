//! Value parsers of the arguments that are counts or times in Unix seconds, which cannot be negative: `--limit`,
//! `--offset`, `--since`, `--start`, `--end` and the like.
//!
//! Each refuses a negative number with clap's own wording, `-1 is not in 0..=<the largest>`, which the terminal and
//! the command API both answer as a usage error. An argument that uses one also sets `allow_negative_numbers`, so
//! that `--limit -1` reaches the parser rather than being read as an option of its own.

use clap::builder::RangedI64ValueParser;

/// A whole number of 0 or more that fits an `i32`.
pub(crate) fn non_negative_i32() -> RangedI64ValueParser<i32> {
    clap::value_parser!(i32).range(0..)
}

/// A whole number of 0 or more that fits an `i64`. Its largest is written out so that the range is worded `0..=<the
/// largest>`, as the other widths' are.
pub(crate) fn non_negative_i64() -> RangedI64ValueParser<i64> {
    clap::value_parser!(i64).range(0..=i64::MAX)
}

/// A whole number of 0 or more that fits a `usize`, worded as [`non_negative_i32`] words it.
pub(crate) fn non_negative_usize(text: &str) -> Result<usize, String> {
    text.parse::<usize>()
        .map_err(|err| match text.parse::<i64>() {
            Ok(number) if number < 0 => format!("{number} is not in 0..={}", usize::MAX),
            _ => err.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::builder::ValueParser;
    use clap::{Arg, Command};

    /// The message clap answers `value` with for an argument that takes `parser`; `None` when it takes it.
    fn refusal(parser: impl Into<ValueParser>, value: &str) -> Option<String> {
        let command = Command::new("t").arg(
            Arg::new("n")
                .long("n")
                .allow_negative_numbers(true)
                .value_parser(parser.into()),
        );
        let arg = format!("--n={value}");
        command
            .try_get_matches_from(["t", arg.as_str()])
            .err()
            .map(|err| err.to_string())
    }

    #[test]
    fn a_negative_number_is_out_of_range_in_every_width() {
        assert!(refusal(non_negative_i32(), "-1")
            .unwrap()
            .contains("-1 is not in 0..=2147483647"));
        assert!(refusal(non_negative_i64(), "-5")
            .unwrap()
            .contains("-5 is not in 0..=9223372036854775807"));
        assert!(refusal(non_negative_usize, "-7")
            .unwrap()
            .contains(&format!("-7 is not in 0..={}", usize::MAX)));
    }

    #[test]
    fn whole_numbers_of_0_or_more_are_taken_and_other_text_is_not() {
        for value in ["0", "7", "+3", "2147483647"] {
            assert_eq!(refusal(non_negative_i32(), value), None, "{value}");
            assert_eq!(refusal(non_negative_i64(), value), None, "{value}");
            assert_eq!(refusal(non_negative_usize, value), None, "{value}");
        }
        for value in ["abc", "", "1.5", "2147483648"] {
            assert!(refusal(non_negative_i32(), value).is_some(), "{value}");
        }
        for value in ["abc", "", "1.5", "-"] {
            assert!(refusal(non_negative_i64(), value).is_some(), "{value}");
            assert!(refusal(non_negative_usize, value).is_some(), "{value}");
        }
    }
}
