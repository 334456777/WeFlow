//! Output language selection for the native CLI.
//!
//! English is the default. Chinese is used only when the environment asks for
//! it, in this order of precedence: `WEFLOW_LANG`, `LC_ALL`, `LC_MESSAGES`,
//! `LANG`, `LANGUAGE` (the first variable that is set and not empty decides).
//! `C`/`POSIX` and any non-Chinese locale resolve to English.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Zh,
}

static CURRENT: OnceLock<Lang> = OnceLock::new();

const ENV_PRECEDENCE: [&str; 5] = ["WEFLOW_LANG", "LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"];

impl Lang {
    /// Parse a locale string such as `zh_CN.UTF-8`, `zh-TW`, `en_US` or `zh`.
    pub fn from_locale(value: &str) -> Lang {
        // LANGUAGE may hold a colon-separated priority list: use its first entry.
        let first = value.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
        if first == "zh" || first.starts_with("zh_") || first.starts_with("zh-") || first.starts_with("zh.") {
            Lang::Zh
        } else {
            Lang::En
        }
    }

    pub fn detect_with(get: impl Fn(&str) -> Option<String>) -> Lang {
        for key in ENV_PRECEDENCE {
            if let Some(value) = get(key) {
                if !value.trim().is_empty() {
                    return Lang::from_locale(&value);
                }
            }
        }
        Lang::En
    }

    pub fn detect() -> Lang {
        Lang::detect_with(|key| std::env::var(key).ok())
    }
}

/// Force the language (used by `--lang`). Must run before the first `current()`.
pub fn set(lang: Lang) {
    let _ = CURRENT.set(lang);
}

pub fn current() -> Lang {
    *CURRENT.get_or_init(Lang::detect)
}

/// Pick the string for the active language.
pub fn tr(en: &'static str, zh: &'static str) -> &'static str {
    match current() {
        Lang::En => en,
        Lang::Zh => zh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn detect(pairs: &[(&str, &str)]) -> Lang {
        let map: HashMap<String, String> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Lang::detect_with(|key| map.get(key).cloned())
    }

    #[test]
    fn defaults_to_english() {
        assert_eq!(detect(&[]), Lang::En);
        assert_eq!(detect(&[("LANG", "C")]), Lang::En);
        assert_eq!(detect(&[("LANG", "POSIX")]), Lang::En);
        assert_eq!(detect(&[("LANG", "en_US.UTF-8")]), Lang::En);
        assert_eq!(detect(&[("LANG", "ja_JP.UTF-8")]), Lang::En);
    }

    #[test]
    fn chinese_locales_select_chinese() {
        for value in ["zh_CN.UTF-8", "zh_TW", "zh-HK", "zh", "ZH_cn"] {
            assert_eq!(detect(&[("LANG", value)]), Lang::Zh, "{value}");
        }
    }

    #[test]
    fn precedence_follows_posix() {
        assert_eq!(detect(&[("LANG", "zh_CN.UTF-8"), ("LC_ALL", "en_US.UTF-8")]), Lang::En);
        assert_eq!(detect(&[("LANG", "en_US"), ("LC_MESSAGES", "zh_CN")]), Lang::Zh);
        assert_eq!(detect(&[("LC_ALL", "C"), ("LANG", "zh_CN")]), Lang::En);
        assert_eq!(detect(&[("LC_ALL", ""), ("LANG", "zh_CN")]), Lang::Zh);
        assert_eq!(detect(&[("WEFLOW_LANG", "en"), ("LANG", "zh_CN")]), Lang::En);
        assert_eq!(detect(&[("LANGUAGE", "zh_CN:en")]), Lang::Zh);
    }
}
