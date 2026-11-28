//! The keys of a profile that WeFlow reads beside its fields, kept in the profile's `extra`: the settings of the AI
//! insights, the message push, the statistics, the image auto-download and the progress bar (#1042, #1149).
//!
//! Each key takes one kind of value. [`Kind::settle`] refuses a value of another kind and gives one it takes the form
//! the readers look for: a switch is a JSON boolean, a count a whole number, a mode its name in lower case. The bounds,
//! names and defaults of the switches, counts, numbers and modes are their descriptions in [`super::keys`].

use serde_json::{Number, Value};

use super::keys::{self, Choice, Count};
use super::{as_bool, as_seconds, as_string};
use crate::error::{AppError, AppResult};

/// What a key takes, and the form it is kept in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Kind {
    /// `true` or `false`, or a word [`as_bool`] takes for them; kept as a JSON boolean.
    Switch,
    /// A whole number of seconds, 0 or more ([`as_seconds`]).
    Seconds,
    /// A whole number in the bounds of the count.
    Count(Count),
    /// A number in the bounds of the number, a fraction or a whole one.
    Number(keys::Number),
    /// One of the names of the choice, in any case; kept in lower case.
    Choice(Choice),
    /// A JSON array of conversation ids.
    Ids,
    /// A JSON object whose keys are conversation ids.
    ById,
    /// Text, kept as it is typed.
    Text,
}

/// Each key and what it takes, in the order `config set --help` lists them.
pub(super) const KEYS: &[(&str, Kind)] = &[
    ("auto_download_whitelist", Kind::Ids),
    (keys::NO_PROGRESS.name, Kind::Switch),
    ("progress_delay_seconds", Kind::Seconds),
    (
        keys::AI_INSIGHT_FILTER_MODE.name,
        Kind::Choice(keys::AI_INSIGHT_FILTER_MODE),
    ),
    ("ai_insight_filter_list", Kind::Ids),
    (
        keys::AI_INSIGHT_SILENCE_DAYS.name,
        Kind::Count(keys::AI_INSIGHT_SILENCE_DAYS),
    ),
    (
        keys::AI_INSIGHT_SCAN_INTERVAL_HOURS.name,
        Kind::Number(keys::AI_INSIGHT_SCAN_INTERVAL_HOURS),
    ),
    (
        keys::AI_INSIGHT_COOLDOWN_MINUTES.name,
        Kind::Count(keys::AI_INSIGHT_COOLDOWN_MINUTES),
    ),
    (keys::AI_INSIGHT_NOTIFICATION_ENABLED.name, Kind::Switch),
    (keys::AI_INSIGHT_ALLOW_CONTEXT.name, Kind::Switch),
    (
        keys::AI_INSIGHT_CONTEXT_COUNT.name,
        Kind::Count(keys::AI_INSIGHT_CONTEXT_COUNT),
    ),
    (keys::AI_INSIGHT_ALLOW_MOMENTS_CONTEXT.name, Kind::Switch),
    ("ai_insight_moments_bindings", Kind::ById),
    (
        keys::AI_INSIGHT_MOMENTS_CONTEXT_COUNT.name,
        Kind::Count(keys::AI_INSIGHT_MOMENTS_CONTEXT_COUNT),
    ),
    (keys::AI_INSIGHT_ALLOW_SOCIAL_CONTEXT.name, Kind::Switch),
    ("ai_insight_weibo_bindings", Kind::ById),
    ("ai_insight_weibo_cookie", Kind::Text),
    (
        keys::AI_INSIGHT_SOCIAL_CONTEXT_COUNT.name,
        Kind::Count(keys::AI_INSIGHT_SOCIAL_CONTEXT_COUNT),
    ),
    ("ai_insight_system_prompt", Kind::Text),
    (keys::AI_INSIGHT_TELEGRAM_ENABLED.name, Kind::Switch),
    ("ai_insight_telegram_token", Kind::Text),
    ("ai_insight_telegram_chat_ids", Kind::Text),
    (keys::AI_FOOTPRINT_ENABLED.name, Kind::Switch),
    ("ai_footprint_system_prompt", Kind::Text),
    (
        keys::MESSAGE_PUSH_FILTER_MODE.name,
        Kind::Choice(keys::MESSAGE_PUSH_FILTER_MODE),
    ),
    ("message_push_filter_list", Kind::Ids),
    ("analytics_excluded_usernames", Kind::Ids),
];

/// What `key` takes; `None` for a key that is not one of [`KEYS`].
pub(super) fn kind_of(key: &str) -> Option<Kind> {
    KEYS.iter().find(|(k, _)| *k == key).map(|(_, kind)| *kind)
}

/// Whether `key` is one of [`KEYS`].
pub(super) fn is_extra_key(key: &str) -> bool {
    kind_of(key).is_some()
}

/// The keys of [`KEYS`] that hold text, which a command line gives as typed rather than reading it as JSON.
pub(super) fn holds_text(key: &str) -> bool {
    kind_of(key) == Some(Kind::Text)
}

impl Kind {
    /// `value` in the form `key` keeps it in.
    ///
    /// # Errors
    /// A usage error that names `key` and what it takes, when `value` is not of this kind.
    pub(super) fn settle(self, value: Value, key: &str) -> AppResult<Value> {
        match self {
            Self::Switch => as_bool(value, key).map(Value::Bool),
            Self::Seconds => as_seconds(value, key).map(Value::from),
            Self::Count(count) => as_count(&value, key, count.min, count.max).map(Value::from),
            Self::Number(number) => as_number(value, key, number.least, number.most),
            Self::Choice(choice) => as_choice(&value, key, &choice),
            Self::Ids => as_ids(value, key),
            Self::ById => as_by_id(value, key),
            Self::Text => as_string(value).map(Value::String),
        }
    }
}

/// `value` as it is shown in an error: text without its quotes.
fn shown(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A whole number from `min` to `max`: a JSON number without a fraction, or text that holds one.
pub(super) fn as_count(value: &Value, key: &str, min: i64, max: i64) -> AppResult<i64> {
    let n = match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    };
    n.filter(|n| (min..=max).contains(n)).ok_or_else(|| {
        let text = shown(value);
        AppError::usage(format!(
            "invalid value '{text}' for {key}; use a whole number from {min} to {max}"
        ))
    })
}

/// A number from `least` to `most`: a JSON number, kept as it is, or text that holds one.
pub(super) fn as_number(value: Value, key: &str, least: f64, most: f64) -> AppResult<Value> {
    let number = match &value {
        Value::Number(n) => Some(n.clone()),
        Value::String(s) => s.trim().parse::<f64>().ok().and_then(Number::from_f64),
        _ => None,
    };
    number
        .filter(|n| n.as_f64().is_some_and(|f| (least..=most).contains(&f)))
        .map(Value::Number)
        .ok_or_else(|| {
            let text = shown(&value);
            AppError::usage(format!(
                "invalid value '{text}' for {key}; use a number from {least} to {most}"
            ))
        })
}

/// One of the names of `choice`, in any case, as its name.
fn as_choice(value: &Value, key: &str, choice: &Choice) -> AppResult<Value> {
    let text = shown(value);
    if let Some(name) = choice.find(&text) {
        return Ok(Value::String(name.to_string()));
    }
    Err(AppError::usage(format!(
        "invalid value '{text}' for {key}; use {}",
        either(choice.names)
    )))
}

/// `names` as a sentence lists them: `a, b or c`.
fn either(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// `value`, or the JSON that text holds, when it is of the type `fits` takes: a list written as text by a WebUI that
/// sent it so is kept as the list.
fn as_json(value: Value, fits: impl Fn(&Value) -> bool) -> Option<Value> {
    let value = match value {
        Value::String(text) => serde_json::from_str(&text).ok()?,
        other => other,
    };
    fits(&value).then_some(value)
}

/// A JSON array of conversation ids.
fn as_ids(value: Value, key: &str) -> AppResult<Value> {
    as_json(value, |v| {
        v.as_array()
            .is_some_and(|ids| ids.iter().all(Value::is_string))
    })
    .ok_or_else(|| {
        AppError::usage(format!(
            "invalid value for {key}; use a JSON array of conversation ids, such as [\"wxid_a\",\"123@chatroom\"]"
        ))
    })
}

/// A JSON object whose keys are conversation ids.
fn as_by_id(value: Value, key: &str) -> AppResult<Value> {
    as_json(value, Value::is_object).ok_or_else(|| {
        AppError::usage(format!(
            "invalid value for {key}; use a JSON object keyed by conversation id"
        ))
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::ConfigStore;

    /// The switches take the words `config set` takes for the other switches, and keep a JSON boolean, which is what
    /// the readers compare with: `yes` was kept as text and read as off (#1149).
    #[test]
    fn a_switch_is_kept_as_a_boolean() {
        let mut store = ConfigStore::default();
        for key in [
            "ai_insight_allow_context",
            "ai_insight_notification_enabled",
            "ai_footprint_enabled",
        ] {
            for (typed, on) in [("yes", true), ("1", true), ("TRUE", true), ("on", true)]
                .into_iter()
                .chain([("no", false), ("0", false), ("off", false)])
            {
                store.set_key_from_text(None, key, typed).unwrap();
                assert_eq!(store.get_key(None, key), json!(on), "{key}={typed}");
            }
            let err = store.set_key_from_text(None, key, "maybe").unwrap_err();
            assert_eq!(err.code, "invalid_arguments");
            assert!(err.message.contains(key), "{}", err.message);
        }
    }

    /// The counts and numbers are refused outside their range and kept as numbers: `-5` silent days made every
    /// conversation silent, `abc` was the default without a word (#1149).
    #[test]
    fn a_number_out_of_its_range_is_refused() {
        let mut store = ConfigStore::default();
        for (key, typed, kept) in [
            ("ai_insight_silence_days", "7", json!(7)),
            ("ai_insight_silence_days", "\"7\"", json!(7)),
            ("ai_insight_cooldown_minutes", "0", json!(0)),
            ("ai_insight_context_count", "80", json!(80)),
            ("ai_insight_moments_context_count", "20", json!(20)),
            ("ai_insight_scan_interval_hours", "0.5", json!(0.5)),
            ("ai_insight_scan_interval_hours", "4", json!(4)),
            ("ai_insight_scan_interval_hours", "\"2.5\"", json!(2.5)),
        ] {
            store.set_key_from_text(None, key, typed).unwrap();
            assert_eq!(store.get_key(None, key), kept, "{key}={typed}");
        }
        for (key, bad) in [
            ("ai_insight_silence_days", "-5"),
            ("ai_insight_silence_days", "0"),
            ("ai_insight_silence_days", "abc"),
            ("ai_insight_silence_days", "1.5"),
            ("ai_insight_cooldown_minutes", "-1"),
            ("ai_insight_context_count", "0"),
            ("ai_insight_moments_context_count", "21"),
            ("ai_insight_social_context_count", "6"),
            ("ai_insight_scan_interval_hours", "-1"),
            ("ai_insight_scan_interval_hours", "0"),
            ("ai_insight_scan_interval_hours", "NaN"),
            ("ai_insight_scan_interval_hours", "[1]"),
        ] {
            let err = store.set_key_from_text(None, key, bad).unwrap_err();
            assert_eq!(err.code, "invalid_arguments", "{key}={bad}");
            assert!(
                err.message.contains(key) && err.message.contains(" from "),
                "{key}={bad}: {}",
                err.message
            );
        }
        // the values that were there stay
        assert_eq!(store.get_key(None, "ai_insight_silence_days"), json!(7));
    }

    /// The modes take their names in any case and keep them in lower case, which is what the readers compare with;
    /// any other name is refused rather than read as the default (#1149).
    #[test]
    fn a_mode_takes_only_its_names() {
        let mut store = ConfigStore::default();
        store
            .set_key_from_text(None, "ai_insight_filter_mode", "Blacklist")
            .unwrap();
        assert_eq!(
            store.get_key(None, "ai_insight_filter_mode"),
            json!("blacklist")
        );
        store
            .set_key_from_text(None, "message_push_filter_mode", " ALL ")
            .unwrap();
        assert_eq!(
            store.get_key(None, "message_push_filter_mode"),
            json!("all")
        );
        let err = store
            .set_key_from_text(None, "ai_insight_filter_mode", "BLACKLISTX")
            .unwrap_err();
        assert!(
            err.message.ends_with("use whitelist or blacklist"),
            "{}",
            err.message
        );
        let err = store
            .set_key_from_text(None, "message_push_filter_mode", "some")
            .unwrap_err();
        assert!(
            err.message.ends_with("use all, whitelist or blacklist"),
            "{}",
            err.message
        );
        assert!(store
            .set_key_from_text(None, "ai_insight_filter_mode", "all")
            .is_err());
    }

    /// The lists and the bindings take JSON, given as JSON or as the text of it; anything else is refused rather than
    /// read as an empty list (#1149).
    #[test]
    fn a_list_takes_a_json_array_of_ids() {
        let mut store = ConfigStore::default();
        for key in [
            "ai_insight_filter_list",
            "message_push_filter_list",
            "analytics_excluded_usernames",
            "auto_download_whitelist",
        ] {
            store
                .set_key_from_text(None, key, r#"["wxid_a","1@chatroom"]"#)
                .unwrap();
            assert_eq!(store.get_key(None, key), json!(["wxid_a", "1@chatroom"]));
            store.set_key(None, key, json!("[\"wxid_b\"]")).unwrap();
            assert_eq!(store.get_key(None, key), json!(["wxid_b"]));
            store.set_key_from_text(None, key, "[]").unwrap();
            for bad in ["wxid_a", "[1]", "{}", "5"] {
                let err = store.set_key_from_text(None, key, bad).unwrap_err();
                assert!(
                    err.message.contains(key) && err.message.contains("JSON array"),
                    "{key}={bad}: {}",
                    err.message
                );
            }
        }
        store
            .set_key_from_text(
                None,
                "ai_insight_moments_bindings",
                r#"{"wxid_a":{"enabled":true}}"#,
            )
            .unwrap();
        assert_eq!(
            store.get_key(None, "ai_insight_moments_bindings"),
            json!({"wxid_a": {"enabled": true}})
        );
        let err = store
            .set_key_from_text(None, "ai_insight_weibo_bindings", "[]")
            .unwrap_err();
        assert!(err.message.contains("JSON object"), "{}", err.message);
    }

    /// The prompts, the token, the cookie and the chat ids are text, kept as typed: Telegram's chat ids of groups are
    /// negative numbers.
    #[test]
    fn a_text_is_kept_as_typed() {
        let mut store = ConfigStore::default();
        for (key, typed) in [
            ("ai_insight_telegram_chat_ids", "-1001234567890"),
            ("ai_insight_telegram_token", "123456:abc"),
            ("ai_insight_system_prompt", "true"),
            ("ai_footprint_system_prompt", "[\"x\"]"),
        ] {
            store.set_key_from_text(None, key, typed).unwrap();
            assert_eq!(store.get_key(None, key), json!(typed), "{key}");
        }
        assert!(store
            .set_key(None, "ai_insight_weibo_cookie", json!(["x"]))
            .is_err());
    }

    /// A file written before `set_key` checked these keys is read with the values in the form it keeps them in; a
    /// value no key takes is left as it is for the file's owner to see.
    #[test]
    fn the_values_of_an_old_file_are_read_as_set_keeps_them() {
        let dir = weflow_native::fixture::TestDir::new("weflow-config-extra-keys");
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            json!({
                "current_profile": "a",
                "profiles": { "a": { "extra": {
                    "ai_insight_allow_context": "yes",
                    "ai_insight_notification_enabled": 0,
                    "ai_insight_context_count": "30",
                    "ai_insight_filter_mode": "BlackList",
                    "message_push_filter_mode": "Whitelist",
                    "message_push_filter_list": "[\"wxid_a\"]",
                    "ai_insight_telegram_chat_ids": -100,
                    "ai_insight_silence_days": -5,
                    "aiInsightCooldownMinutes": 30,
                    "my_own": "yes"
                } } }
            })
            .to_string(),
        )
        .unwrap();
        let store = ConfigStore::load(&path).unwrap();
        for (key, read) in [
            ("ai_insight_allow_context", json!(true)),
            ("ai_insight_notification_enabled", json!(false)),
            ("ai_insight_context_count", json!(30)),
            ("ai_insight_filter_mode", json!("blacklist")),
            ("message_push_filter_mode", json!("whitelist")),
            ("message_push_filter_list", json!(["wxid_a"])),
            ("ai_insight_telegram_chat_ids", json!("-100")),
            ("ai_insight_silence_days", json!(-5)),
            ("ai_insight_cooldown_minutes", json!(30)),
            ("my_own", json!("yes")),
        ] {
            assert_eq!(store.get_key(None, key), read, "{key}");
        }
    }

    /// Every key is snake case, listed once, and none is a field of the profile.
    #[test]
    fn every_key_is_listed_once() {
        for (i, (key, _)) in KEYS.iter().enumerate() {
            assert!(
                key.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{key}"
            );
            assert!(!KEYS[..i].iter().any(|(k, _)| k == key), "{key}");
            assert!(!super::super::is_profile_key(key), "{key}");
        }
    }
}
