//! The keys of the config file that take a switch, a bounded number or one of some names, each described once (#1265):
//! what it takes and what it is read as when it is not set.
//!
//! `config set` checks a value against the key's description here, and the readers (the insights, the message push,
//! `serve`, the progress bar) read it through [`Setting::read`], which gives the key's default for a value the key
//! does not take, such as one in a file edited by hand. A reader therefore keeps no default or bounds of its own.

use std::net::IpAddr;

use serde_json::Value;

use super::extra_keys::{as_count, as_number};
use crate::model_api::ModelApiFormat;

/// A key read as a value of its own type.
pub trait Setting {
    /// What the key is read as.
    type Value;

    /// The key's name, as `config set` takes it.
    fn name(&self) -> &'static str;

    /// `value`, the key's entry in the config file, as the key reads it: what `config set` keeps, or the key's
    /// default when it is not set or holds what the key does not take.
    fn read(&self, value: &Value) -> Self::Value;
}

/// A switch, `true` or `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switch {
    /// The key.
    pub name: &'static str,
    /// What it is read as when it is not set.
    pub default: bool,
}

impl Setting for Switch {
    type Value = bool;

    fn name(&self) -> &'static str {
        self.name
    }

    fn read(&self, value: &Value) -> bool {
        super::as_bool(value.clone(), self.name).unwrap_or(self.default)
    }
}

/// A whole number from `min` to `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Count {
    /// The key.
    pub name: &'static str,
    /// The least number taken.
    pub min: i64,
    /// The greatest number taken.
    pub max: i64,
    /// What it is read as when it is not set.
    pub default: i64,
}

impl Setting for Count {
    type Value = i64;

    fn name(&self) -> &'static str {
        self.name
    }

    fn read(&self, value: &Value) -> i64 {
        as_count(value, self.name, self.min, self.max).unwrap_or(self.default)
    }
}

/// A whole number from `min` to `max` that the config gives no default: its reader says what none means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// The key.
    pub name: &'static str,
    /// The least number taken.
    pub min: i64,
    /// The greatest number taken.
    pub max: i64,
}

/// A number from `least` to `most`, a fraction or a whole one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Number {
    /// The key.
    pub name: &'static str,
    /// The least number taken.
    pub least: f64,
    /// The greatest number taken.
    pub most: f64,
    /// What it is read as when it is not set.
    pub default: f64,
}

impl Setting for Number {
    type Value = f64;

    fn name(&self) -> &'static str {
        self.name
    }

    fn read(&self, value: &Value) -> f64 {
        as_number(value.clone(), self.name, self.least, self.most)
            .ok()
            .and_then(|n| n.as_f64())
            .unwrap_or(self.default)
    }
}

/// One of some names, given in any case and kept in lower case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// The key.
    pub name: &'static str,
    /// The names taken.
    pub names: &'static [&'static str],
    /// What it is read as when it is not set; one of [`Self::names`].
    pub default: &'static str,
}

impl Choice {
    /// The name `typed` is, in any case and without the spaces around it; `None` when it is none of them.
    pub(super) fn find(&self, typed: &str) -> Option<&'static str> {
        let typed = typed.trim().to_ascii_lowercase();
        self.names.iter().copied().find(|name| *name == typed)
    }
}

impl Setting for Choice {
    type Value = &'static str;

    fn name(&self) -> &'static str {
        self.name
    }

    fn read(&self, value: &Value) -> &'static str {
        value
            .as_str()
            .and_then(|typed| self.find(typed))
            .unwrap_or(self.default)
    }
}

/// The API format of the model API, one of [`ModelApiFormat::NAMES`].
///
/// The profile keeps it as text, so that a file edited by hand still loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiFormat {
    /// The key.
    pub name: &'static str,
}

impl Setting for ApiFormat {
    type Value = ModelApiFormat;

    fn name(&self) -> &'static str {
        self.name
    }

    fn read(&self, value: &Value) -> ModelApiFormat {
        value
            .as_str()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or_default()
    }
}

/// A listen address, as `serve_host` and `serve --host` take it (#1265).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenHost<'a> {
    /// An IP address.
    Ip(IpAddr),
    /// A host name, which `serve` resolves when it starts.
    Name(&'a str),
}

/// `host` as a listen address: an IPv4 or IPv6 address, the latter with or without brackets (`::1`, `[::1]`), or a host
/// name.
///
/// # Errors
/// Why it is neither, to follow the address in an error.
pub fn listen_host(host: &str) -> Result<ListenHost<'_>, &'static str> {
    let host = host.trim();
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return Ok(ListenHost::Ip(ip));
    }
    if bare.is_empty() || bare.contains(['[', ']', '/']) {
        return Err("not an IP address or a host name");
    }
    Ok(ListenHost::Name(bare))
}

/// `no_progress`: whether the automatic progress bar is hidden.
pub const NO_PROGRESS: Switch = Switch {
    name: "no_progress",
    default: false,
};

/// `image_xor_key`: the image XOR key.
pub const IMAGE_XOR_KEY: Bounds = Bounds {
    name: "image_xor_key",
    min: 0,
    max: 255,
};

/// `ai_model_api_format`: the API format of the model API; `openai` unless it names another.
pub const AI_MODEL_API_FORMAT: ApiFormat = ApiFormat {
    name: "ai_model_api_format",
};

/// `ai_model_api_max_tokens`: the most tokens of one answer, up to 2,000,000, far above what a model answers with. The
/// default leaves room for a short answer and for the reasoning a model may do first, which counts against the same
/// limit.
pub const AI_MODEL_API_MAX_TOKENS: Count = Count {
    name: "ai_model_api_max_tokens",
    min: 1,
    max: 2_000_000,
    default: 4096,
};

/// `ai_insight_enabled`: whether the AI insights run.
pub const AI_INSIGHT_ENABLED: Switch = Switch {
    name: "ai_insight_enabled",
    default: false,
};

/// `ai_insight_filter_mode`: the insights are for the conversations of the list only, or for all but them.
pub const AI_INSIGHT_FILTER_MODE: Choice = Choice {
    name: "ai_insight_filter_mode",
    names: &["whitelist", "blacklist"],
    default: "whitelist",
};

/// `ai_insight_silence_days`: the days without a message before a conversation counts as silent.
pub const AI_INSIGHT_SILENCE_DAYS: Count = Count {
    name: "ai_insight_silence_days",
    min: 1,
    max: 365,
    default: 3,
};

/// `ai_insight_scan_interval_hours`: the hours between two scans for silent conversations.
pub const AI_INSIGHT_SCAN_INTERVAL_HOURS: Number = Number {
    name: "ai_insight_scan_interval_hours",
    least: 0.1,
    most: 168.0,
    default: 4.0,
};

/// `ai_insight_cooldown_minutes`: the minutes before new messages of a conversation are asked about again; 0 does not
/// wait.
pub const AI_INSIGHT_COOLDOWN_MINUTES: Count = Count {
    name: "ai_insight_cooldown_minutes",
    min: 0,
    max: 10_080,
    default: 120,
};

/// `ai_insight_notification_enabled`: whether `serve --insight` prints the insights it writes.
pub const AI_INSIGHT_NOTIFICATION_ENABLED: Switch = Switch {
    name: "ai_insight_notification_enabled",
    default: true,
};

/// `ai_insight_allow_context`: whether the recent messages go to the model with the question.
pub const AI_INSIGHT_ALLOW_CONTEXT: Switch = Switch {
    name: "ai_insight_allow_context",
    default: false,
};

/// `ai_insight_context_count`: the recent messages sent with the question.
pub const AI_INSIGHT_CONTEXT_COUNT: Count = Count {
    name: "ai_insight_context_count",
    min: 1,
    max: 1000,
    default: 40,
};

/// `ai_insight_allow_moments_context`: whether the Moments of the bound conversations go to the model.
pub const AI_INSIGHT_ALLOW_MOMENTS_CONTEXT: Switch = Switch {
    name: "ai_insight_allow_moments_context",
    default: false,
};

/// `ai_insight_moments_context_count`: the Moments posts sent.
pub const AI_INSIGHT_MOMENTS_CONTEXT_COUNT: Count = Count {
    name: "ai_insight_moments_context_count",
    min: 1,
    max: 20,
    default: 5,
};

/// `ai_insight_allow_social_context`: whether the public Weibo posts of the bound UIDs go to the model.
pub const AI_INSIGHT_ALLOW_SOCIAL_CONTEXT: Switch = Switch {
    name: "ai_insight_allow_social_context",
    default: false,
};

/// `ai_insight_social_context_count`: the Weibo posts sent.
pub const AI_INSIGHT_SOCIAL_CONTEXT_COUNT: Count = Count {
    name: "ai_insight_social_context_count",
    min: 1,
    max: 5,
    default: 3,
};

/// `ai_insight_telegram_enabled`: whether the insights are sent to Telegram too.
pub const AI_INSIGHT_TELEGRAM_ENABLED: Switch = Switch {
    name: "ai_insight_telegram_enabled",
    default: false,
};

/// `ai_footprint_enabled`: whether `insight footprint-summary` asks the model.
pub const AI_FOOTPRINT_ENABLED: Switch = Switch {
    name: "ai_footprint_enabled",
    default: false,
};

/// `message_push_filter_mode`: push every conversation, those of the list only, or all but them.
pub const MESSAGE_PUSH_FILTER_MODE: Choice = Choice {
    name: "message_push_filter_mode",
    names: &["all", "whitelist", "blacklist"],
    default: "all",
};

/// `serve_port`: the listen port of `serve`.
pub const SERVE_PORT: Bounds = Bounds {
    name: "serve_port",
    min: 1,
    max: u16::MAX as i64,
};

/// `serve_max_tasks`: the tasks `serve` runs at once.
pub const SERVE_MAX_TASKS: Bounds = Bounds {
    name: "serve_max_tasks",
    min: 1,
    max: u16::MAX as i64,
};

/// `serve_max_reads`: the other commands `serve` runs at once.
pub const SERVE_MAX_READS: Bounds = Bounds {
    name: "serve_max_reads",
    min: 1,
    max: u16::MAX as i64,
};

/// `serve_max_media`: the images and voices `serve` decodes at once.
pub const SERVE_MAX_MEDIA: Bounds = Bounds {
    name: "serve_max_media",
    min: 1,
    max: u16::MAX as i64,
};

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::extra_keys::{self, Kind};
    use crate::config::ConfigStore;

    /// Every key described here, so that a test can go through them.
    const DESCRIBED: &[&str] = &[
        NO_PROGRESS.name,
        IMAGE_XOR_KEY.name,
        AI_MODEL_API_FORMAT.name,
        AI_MODEL_API_MAX_TOKENS.name,
        AI_INSIGHT_ENABLED.name,
        AI_INSIGHT_FILTER_MODE.name,
        AI_INSIGHT_SILENCE_DAYS.name,
        AI_INSIGHT_SCAN_INTERVAL_HOURS.name,
        AI_INSIGHT_COOLDOWN_MINUTES.name,
        AI_INSIGHT_NOTIFICATION_ENABLED.name,
        AI_INSIGHT_ALLOW_CONTEXT.name,
        AI_INSIGHT_CONTEXT_COUNT.name,
        AI_INSIGHT_ALLOW_MOMENTS_CONTEXT.name,
        AI_INSIGHT_MOMENTS_CONTEXT_COUNT.name,
        AI_INSIGHT_ALLOW_SOCIAL_CONTEXT.name,
        AI_INSIGHT_SOCIAL_CONTEXT_COUNT.name,
        AI_INSIGHT_TELEGRAM_ENABLED.name,
        AI_FOOTPRINT_ENABLED.name,
        MESSAGE_PUSH_FILTER_MODE.name,
        SERVE_PORT.name,
        SERVE_MAX_TASKS.name,
        SERVE_MAX_READS.name,
        SERVE_MAX_MEDIA.name,
    ];

    /// Every description names a key `config set` lists, and the table of the profile's other keys checks each of
    /// them by its description rather than by bounds or names of its own.
    #[test]
    fn every_description_is_a_key_config_set_takes() {
        for name in DESCRIBED {
            assert!(crate::config::is_known_key(name), "{name}");
        }
        for (key, kind) in extra_keys::KEYS {
            let described = match kind {
                Kind::Count(count) => Some(count.name),
                Kind::Number(number) => Some(number.name),
                Kind::Choice(choice) => Some(choice.name),
                _ => None,
            };
            if let Some(name) = described {
                assert_eq!(name, *key);
            }
            if matches!(
                kind,
                Kind::Count(_) | Kind::Number(_) | Kind::Choice(_) | Kind::Switch
            ) {
                assert!(DESCRIBED.contains(key), "{key} has no description");
            }
        }
        for choice in [AI_INSIGHT_FILTER_MODE, MESSAGE_PUSH_FILTER_MODE] {
            assert!(choice.names.contains(&choice.default), "{}", choice.name);
        }
        for count in [
            AI_MODEL_API_MAX_TOKENS,
            AI_INSIGHT_SILENCE_DAYS,
            AI_INSIGHT_COOLDOWN_MINUTES,
            AI_INSIGHT_CONTEXT_COUNT,
            AI_INSIGHT_MOMENTS_CONTEXT_COUNT,
            AI_INSIGHT_SOCIAL_CONTEXT_COUNT,
        ] {
            assert!(
                (count.min..=count.max).contains(&count.default),
                "{}",
                count.name
            );
        }
    }

    /// A key reads what `config set` keeps, and its default when it is not set or holds what the key does not take,
    /// as a file edited by hand may.
    #[test]
    fn a_setting_reads_what_set_keeps_else_its_default() {
        let mut store = ConfigStore::default();
        let hand_written = |store: &mut ConfigStore, key: &str, value: Value| {
            store.profile_mut(None).extra.insert(key.to_string(), value);
        };

        assert_eq!(store.setting(None, &AI_INSIGHT_SILENCE_DAYS), 3);
        store
            .set_key_from_text(None, AI_INSIGHT_SILENCE_DAYS.name, "7")
            .unwrap();
        assert_eq!(store.setting(None, &AI_INSIGHT_SILENCE_DAYS), 7);
        for bad in [
            json!(0),
            json!(-5),
            json!(1.5),
            json!("abc"),
            json!(366),
            json!(null),
        ] {
            hand_written(&mut store, AI_INSIGHT_SILENCE_DAYS.name, bad.clone());
            assert_eq!(store.setting(None, &AI_INSIGHT_SILENCE_DAYS), 3, "{bad}");
        }
        hand_written(&mut store, AI_INSIGHT_SILENCE_DAYS.name, json!(" 9 "));
        assert_eq!(store.setting(None, &AI_INSIGHT_SILENCE_DAYS), 9);

        assert_eq!(store.setting(None, &AI_INSIGHT_SCAN_INTERVAL_HOURS), 4.0);
        store
            .set_key_from_text(None, AI_INSIGHT_SCAN_INTERVAL_HOURS.name, "0.5")
            .unwrap();
        assert_eq!(store.setting(None, &AI_INSIGHT_SCAN_INTERVAL_HOURS), 0.5);
        for bad in [json!(0), json!(0.05), json!(169), json!("x")] {
            hand_written(&mut store, AI_INSIGHT_SCAN_INTERVAL_HOURS.name, bad.clone());
            assert_eq!(
                store.setting(None, &AI_INSIGHT_SCAN_INTERVAL_HOURS),
                4.0,
                "{bad}"
            );
        }

        assert!(store.setting(None, &AI_INSIGHT_NOTIFICATION_ENABLED));
        assert!(!store.setting(None, &AI_INSIGHT_ALLOW_CONTEXT));
        store
            .set_key_from_text(None, AI_INSIGHT_NOTIFICATION_ENABLED.name, "off")
            .unwrap();
        assert!(!store.setting(None, &AI_INSIGHT_NOTIFICATION_ENABLED));
        hand_written(&mut store, AI_INSIGHT_ALLOW_CONTEXT.name, json!("maybe"));
        assert!(!store.setting(None, &AI_INSIGHT_ALLOW_CONTEXT));

        assert_eq!(store.setting(None, &MESSAGE_PUSH_FILTER_MODE), "all");
        store
            .set_key_from_text(None, MESSAGE_PUSH_FILTER_MODE.name, "Blacklist")
            .unwrap();
        assert_eq!(store.setting(None, &MESSAGE_PUSH_FILTER_MODE), "blacklist");
        hand_written(
            &mut store,
            AI_INSIGHT_FILTER_MODE.name,
            json!(" BLACKLIST "),
        );
        assert_eq!(store.setting(None, &AI_INSIGHT_FILTER_MODE), "blacklist");
        hand_written(&mut store, AI_INSIGHT_FILTER_MODE.name, json!("all"));
        assert_eq!(store.setting(None, &AI_INSIGHT_FILTER_MODE), "whitelist");

        assert_eq!(
            store.setting(None, &AI_MODEL_API_FORMAT),
            ModelApiFormat::default()
        );
        store
            .set_key_from_text(None, AI_MODEL_API_FORMAT.name, "anthropic")
            .unwrap();
        assert_eq!(
            store.setting(None, &AI_MODEL_API_FORMAT),
            ModelApiFormat::Anthropic
        );
        store.profile_mut(None).ai_model_api_format = Some("gemini".into());
        assert_eq!(
            store.setting(None, &AI_MODEL_API_FORMAT),
            ModelApiFormat::default()
        );

        assert_eq!(store.setting(None, &AI_MODEL_API_MAX_TOKENS), 4096);
        store
            .set_key_from_text(None, AI_MODEL_API_MAX_TOKENS.name, "512")
            .unwrap();
        assert_eq!(store.setting(None, &AI_MODEL_API_MAX_TOKENS), 512);
        store
            .set_key_from_text(None, AI_MODEL_API_MAX_TOKENS.name, "2000000")
            .unwrap();
        assert_eq!(store.setting(None, &AI_MODEL_API_MAX_TOKENS), 2_000_000);
        for refused in ["0", "2000001", "abc"] {
            assert!(
                store
                    .set_key_from_text(None, AI_MODEL_API_MAX_TOKENS.name, refused)
                    .is_err(),
                "{refused}"
            );
        }
    }

    /// A listen address is an IP address, an IPv6 one with or without brackets, or a host name.
    #[test]
    fn a_listen_host_is_an_address_or_a_name() {
        let ip = |text: &str| ListenHost::Ip(text.parse().unwrap());
        assert_eq!(listen_host("127.0.0.1"), Ok(ip("127.0.0.1")));
        assert_eq!(listen_host(" 0.0.0.0 "), Ok(ip("0.0.0.0")));
        assert_eq!(listen_host("::1"), Ok(ip("::1")));
        assert_eq!(listen_host("[::1]"), Ok(ip("::1")));
        assert_eq!(listen_host("localhost"), Ok(ListenHost::Name("localhost")));
        for bad in ["", "  ", "[localhost", "a/b", "[::1"] {
            assert!(listen_host(bad).is_err(), "{bad:?}");
        }
    }
}
