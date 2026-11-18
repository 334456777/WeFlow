//! Which failures are tried again, and after what pause, as the official SDKs of both services do: a lost connection,
//! 408, 409, 429 and the 5xx statuses (Anthropic's 529 among them), twice, after the pause the service asks for in
//! `retry-after-ms` or `retry-after`, else after a short exponential backoff. A quota or a credit that is used up is
//! not tried again: waiting does not restore it.
use std::time::Duration;

use reqwest::header::{HeaderMap, RETRY_AFTER};

use super::{ModelApiFormat, StatusError};

/// The requests tried again after the first one, as both SDKs' `max_retries`.
pub(super) const MAX_RETRIES: u32 = 2;

/// The pause before the first retry when the service asks for none; it doubles each time.
const FIRST_BACKOFF: Duration = Duration::from_millis(500);

/// The longest pause of the backoff.
const LONGEST_BACKOFF: Duration = Duration::from_secs(8);

/// The longest pause a service may ask for and still be waited for; a longer one ends the request with its error,
/// which says to try later.
const LONGEST_REQUESTED: Duration = Duration::from_secs(60);

/// Whether a 429 says that a quota, a credit or a spend limit is used up, which waiting does not restore: its error
/// names one, or it is Anthropic's, which for a spend limit carries no `retry-after`. `asked`: the answer gave a pause.
pub(super) fn used_up(format: ModelApiFormat, failure: &StatusError, asked: bool) -> bool {
    failure.status == 429
        && (failure.out_of_quota() || (format == ModelApiFormat::Anthropic && !asked))
}

/// Whether the failure is a transient one, worth trying again.
pub(super) fn worth_retrying(failure: &StatusError) -> bool {
    match failure.status {
        408 | 409 | 500..=599 => true,
        429 => !failure.used_up,
        _ => false,
    }
}

/// The pause the answer asks for: `retry-after-ms` (milliseconds), else `retry-after` (seconds, or an HTTP date).
pub(super) fn requested_wait(headers: &HeaderMap) -> Option<Duration> {
    let text = |name| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
    };
    if let Some(ms) = text("retry-after-ms").and_then(|v| v.parse::<f64>().ok()) {
        return seconds(ms / 1000.0);
    }
    let after = text(RETRY_AFTER.as_str())?;
    if let Ok(secs) = after.parse::<f64>() {
        return seconds(secs);
    }
    let at = chrono::DateTime::parse_from_rfc2822(after).ok()?;
    let ms = (at.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_milliseconds();
    Some(Duration::from_millis(u64::try_from(ms).unwrap_or(0)))
}

fn seconds(secs: f64) -> Option<Duration> {
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs.min(1e9)))
}

/// The pause before retry number `retries + 1`: the one asked for, unless it is longer than [`LONGEST_REQUESTED`]
/// (`None`: do not retry), else the backoff.
pub(super) fn pause(asked: Option<Duration>, retries: u32) -> Option<Duration> {
    match asked {
        Some(wait) if wait > LONGEST_REQUESTED => None,
        Some(wait) => Some(wait),
        None => Some(backoff(retries)),
    }
}

/// The backoff before retry number `retries + 1`: [`FIRST_BACKOFF`] doubled each time up to [`LONGEST_BACKOFF`], less
/// up to a quarter at random, so that clients that failed together do not retry together.
pub(super) fn backoff(retries: u32) -> Duration {
    let full = FIRST_BACKOFF
        .saturating_mul(2u32.saturating_pow(retries))
        .min(LONGEST_BACKOFF);
    full.mul_f64(1.0 - 0.25 * rand::random::<f64>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn failure(status: u16, body: &str) -> StatusError {
        StatusError::read(status, body, None)
    }

    /// `failure`, marked used up as the request marks it.
    fn read(format: ModelApiFormat, status: u16, body: &str, asked: bool) -> StatusError {
        let mut failure = failure(status, body);
        failure.used_up = used_up(format, &failure, asked);
        failure
    }

    #[test]
    fn transient_statuses_are_retried() {
        for status in [408, 409, 429, 500, 502, 503, 529] {
            assert!(
                worth_retrying(&read(ModelApiFormat::OpenAi, status, "{}", false)),
                "{status}"
            );
        }
        for status in [400, 401, 402, 403, 404, 413, 422] {
            assert!(
                !worth_retrying(&read(ModelApiFormat::OpenAi, status, "{}", true)),
                "{status}"
            );
        }
    }

    #[test]
    fn a_used_up_quota_is_not_retried() {
        let quota = r#"{"error":{"message":"quota","type":"insufficient_quota","code":"insufficient_quota"}}"#;
        assert!(!worth_retrying(&read(
            ModelApiFormat::OpenAi,
            429,
            quota,
            true
        )));
        // Anthropic's spend limit: a 429 without retry-after
        let limit = r#"{"type":"error","error":{"type":"rate_limit_error","message":"limit"}}"#;
        let spent = read(ModelApiFormat::Anthropic, 429, limit, false);
        assert!(spent.used_up && !worth_retrying(&spent));
        assert!(spent.to_string().contains("used up"), "{spent}");
        assert!(worth_retrying(&read(
            ModelApiFormat::Anthropic,
            429,
            limit,
            true
        )));
    }

    #[test]
    fn the_requested_pause_is_read_and_bounded() {
        let mut headers = HeaderMap::new();
        assert_eq!(requested_wait(&headers), None);
        headers.insert(RETRY_AFTER, HeaderValue::from_static("2"));
        assert_eq!(requested_wait(&headers), Some(Duration::from_secs(2)));
        headers.insert("retry-after-ms", HeaderValue::from_static("150"));
        assert_eq!(requested_wait(&headers), Some(Duration::from_millis(150)));
        headers.remove("retry-after-ms");
        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        assert_eq!(
            requested_wait(&headers),
            Some(Duration::ZERO),
            "a date gone by"
        );
        headers.insert(RETRY_AFTER, HeaderValue::from_static("soon"));
        assert_eq!(requested_wait(&headers), None);

        assert_eq!(pause(Some(Duration::from_secs(61)), 0), None);
        assert_eq!(pause(Some(Duration::ZERO), 1), Some(Duration::ZERO));
    }

    #[test]
    fn the_backoff_doubles_up_to_its_limit() {
        for retries in 0..10 {
            let full = FIRST_BACKOFF
                .saturating_mul(2u32.saturating_pow(retries))
                .min(LONGEST_BACKOFF);
            let wait = backoff(retries);
            assert!(
                wait <= full && wait >= full.mul_f64(0.75),
                "{retries}: {wait:?}"
            );
        }
    }
}
