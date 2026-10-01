//! Output language selection for the native CLI.
//!
//! The language follows the system. Chinese is used when the locale asks for it, English otherwise
//! (English and Chinese are the only translations). In order of precedence:
//!
//! 1. `--lang en|zh` (set by the command line parser);
//! 2. the environment: `WEFLOW_LANG`, `LC_ALL`, `LC_MESSAGES`, `LANG`, `LANGUAGE` (the first variable that is set
//!    and not empty decides; `C`/`POSIX` and any non-Chinese locale resolve to English);
//! 3. when none of those is set (typical on Windows), the operating system's display language:
//!    Windows `GetUserDefaultUILanguage`, macOS `AppleLanguages`;
//! 4. English.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Zh,
}

static CURRENT: OnceLock<Lang> = OnceLock::new();

const ENV_PRECEDENCE: [&str; 5] = ["WEFLOW_LANG", "LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"];

impl Lang {
    /// Parse a locale string such as `zh_CN.UTF-8`, `zh-TW`, `zh-Hans-CN`, `en_US` or `zh`.
    pub fn from_locale(value: &str) -> Lang {
        // LANGUAGE may hold a colon-separated priority list: use its first entry.
        let first = value.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
        if first == "zh" || first.starts_with("zh_") || first.starts_with("zh-") || first.starts_with("zh.") {
            Lang::Zh
        } else {
            Lang::En
        }
    }

    /// The environment decides when any of the variables is set; otherwise `system` (the OS display language).
    pub fn detect_with(get: impl Fn(&str) -> Option<String>, system: impl Fn() -> Option<String>) -> Lang {
        for key in ENV_PRECEDENCE {
            if let Some(value) = get(key) {
                if !value.trim().is_empty() {
                    return Lang::from_locale(&value);
                }
            }
        }
        system().map_or(Lang::En, |s| Lang::from_locale(&s))
    }

    pub fn detect() -> Lang {
        Lang::detect_with(|key| std::env::var(key).ok(), system_locale)
    }
}

/// The operating system's display language as a locale string (`zh-CN`, `en-US`, ...), if it can be determined.
pub fn system_locale() -> Option<String> {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetUserDefaultUILanguage() -> u16;
        }
        // SAFETY: a plain Win32 call without arguments.
        let langid = unsafe { GetUserDefaultUILanguage() };
        // The primary language lives in the low 10 bits; 0x04 is Chinese (all scripts and regions).
        return Some(if langid & 0x3ff == 0x04 { "zh".to_string() } else { "other".to_string() });
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("defaults").args(["read", "-g", "AppleLanguages"]).output().ok()?;
        return first_apple_language(&String::from_utf8_lossy(&out.stdout));
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        None
    }
}

/// First entry of `defaults read -g AppleLanguages` output, e.g. `(\n    "zh-Hans-CN",\n    "en-CN"\n)`.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn first_apple_language(output: &str) -> Option<String> {
    output
        .lines()
        .map(|l| l.trim().trim_end_matches(',').trim_matches('"'))
        .find(|l| !l.is_empty() && *l != "(" && *l != ")")
        .map(str::to_string)
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

    fn detect(pairs: &[(&str, &str)], system: Option<&str>) -> Lang {
        let map: HashMap<String, String> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Lang::detect_with(|key| map.get(key).cloned(), || system.map(str::to_string))
    }

    #[test]
    fn unknown_locales_resolve_to_english() {
        assert_eq!(detect(&[], None), Lang::En, "nothing to go on");
        assert_eq!(detect(&[("LANG", "C")], Some("zh-CN")), Lang::En, "an explicit C locale is respected");
        assert_eq!(detect(&[("LANG", "POSIX")], None), Lang::En);
        assert_eq!(detect(&[("LANG", "en_US.UTF-8")], None), Lang::En);
        assert_eq!(detect(&[("LANG", "ja_JP.UTF-8")], None), Lang::En);
    }

    #[test]
    fn chinese_locales_select_chinese() {
        for value in ["zh_CN.UTF-8", "zh_TW", "zh-HK", "zh", "ZH_cn", "zh-Hans-CN"] {
            assert_eq!(detect(&[("LANG", value)], None), Lang::Zh, "{value}");
        }
    }

    #[test]
    fn precedence_follows_posix() {
        assert_eq!(detect(&[("LANG", "zh_CN.UTF-8"), ("LC_ALL", "en_US.UTF-8")], None), Lang::En);
        assert_eq!(detect(&[("LANG", "en_US"), ("LC_MESSAGES", "zh_CN")], None), Lang::Zh);
        assert_eq!(detect(&[("LC_ALL", "C"), ("LANG", "zh_CN")], None), Lang::En);
        assert_eq!(detect(&[("LC_ALL", ""), ("LANG", "zh_CN")], None), Lang::Zh);
        assert_eq!(detect(&[("WEFLOW_LANG", "en"), ("LANG", "zh_CN")], Some("zh-CN")), Lang::En);
        assert_eq!(detect(&[("LANGUAGE", "zh_CN:en")], None), Lang::Zh);
    }

    #[test]
    fn follows_the_system_when_the_environment_is_silent() {
        assert_eq!(detect(&[], Some("zh-CN")), Lang::Zh);
        assert_eq!(detect(&[], Some("zh-Hans-CN")), Lang::Zh);
        assert_eq!(detect(&[], Some("zh")), Lang::Zh);
        assert_eq!(detect(&[], Some("en-US")), Lang::En);
        assert_eq!(detect(&[], Some("other")), Lang::En);
        assert_eq!(detect(&[("LC_ALL", "  ")], Some("zh-TW")), Lang::Zh, "blank variables do not count as set");
        // any environment variable beats the system language, in either direction
        assert_eq!(detect(&[("LANG", "en_US.UTF-8")], Some("zh-CN")), Lang::En);
        assert_eq!(detect(&[("WEFLOW_LANG", "zh")], Some("en-US")), Lang::Zh);
    }

    #[test]
    fn apple_languages_output_is_parsed() {
        assert_eq!(first_apple_language("(\n    \"zh-Hans-CN\",\n    \"en-CN\"\n)\n").as_deref(), Some("zh-Hans-CN"));
        assert_eq!(first_apple_language("(\n    en\n)").as_deref(), Some("en"));
        assert_eq!(first_apple_language(""), None);
        assert_eq!(first_apple_language("(\n)"), None);
    }
}
