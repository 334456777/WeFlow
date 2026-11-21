//! The names WeFlow's desktop app gave the config keys, which WeFlow 7.0.0 and earlier took as well (#1021). A config
//! file that holds them is read with the keys' own names, and a command naming one is told the key's name instead.

use crate::error::{AppError, AppResult};

/// Each name of the desktop app, and the key it stands for.
pub(super) const RENAMED: &[(&str, &str)] = &[
    ("dbPath", "db_path"),
    ("decryptKey", "decrypt_key"),
    ("myWxid", "wxid"),
    ("imageXorKey", "image_xor_key"),
    ("imageAesKey", "image_aes_key"),
    ("cachePath", "cache_path"),
    ("httpApiToken", "http_api_token"),
    ("aiModelApiFormat", "ai_model_api_format"),
    ("aiModelApiBaseUrl", "ai_model_api_base_url"),
    ("aiModelApiKey", "ai_model_api_key"),
    ("aiModelApiModel", "ai_model_api_model"),
    ("aiModelApiMaxTokens", "ai_model_api_max_tokens"),
    ("aiInsightApiBaseUrl", "ai_model_api_base_url"),
    ("aiInsightApiKey", "ai_model_api_key"),
    ("aiInsightApiModel", "ai_model_api_model"),
    ("aiInsightEnabled", "ai_insight_enabled"),
    ("aiInsightAllowContext", "ai_insight_allow_context"),
    ("aiInsightContextCount", "ai_insight_context_count"),
    (
        "aiInsightAllowMomentsContext",
        "ai_insight_allow_moments_context",
    ),
    (
        "aiInsightMomentsContextCount",
        "ai_insight_moments_context_count",
    ),
    ("aiInsightMomentsBindings", "ai_insight_moments_bindings"),
    (
        "aiInsightAllowSocialContext",
        "ai_insight_allow_social_context",
    ),
    (
        "aiInsightSocialContextCount",
        "ai_insight_social_context_count",
    ),
    ("aiInsightWeiboBindings", "ai_insight_weibo_bindings"),
    ("aiInsightWeiboCookie", "ai_insight_weibo_cookie"),
    ("aiInsightSilenceDays", "ai_insight_silence_days"),
    ("aiInsightCooldownMinutes", "ai_insight_cooldown_minutes"),
    (
        "aiInsightScanIntervalHours",
        "ai_insight_scan_interval_hours",
    ),
    ("aiInsightFilterMode", "ai_insight_filter_mode"),
    ("aiInsightFilterList", "ai_insight_filter_list"),
    (
        "aiInsightNotificationEnabled",
        "ai_insight_notification_enabled",
    ),
    ("aiInsightTelegramEnabled", "ai_insight_telegram_enabled"),
    ("aiInsightTelegramToken", "ai_insight_telegram_token"),
    ("aiInsightTelegramChatIds", "ai_insight_telegram_chat_ids"),
    ("aiInsightSystemPrompt", "ai_insight_system_prompt"),
    ("aiFootprintEnabled", "ai_footprint_enabled"),
    ("aiFootprintSystemPrompt", "ai_footprint_system_prompt"),
    ("messagePushFilterMode", "message_push_filter_mode"),
    ("messagePushFilterList", "message_push_filter_list"),
    ("autoDownloadWhitelist", "auto_download_whitelist"),
    ("analyticsExcludedUsernames", "analytics_excluded_usernames"),
    ("progressDelaySeconds", "progress_delay_seconds"),
    ("currentProfile", "current_profile"),
    ("configPath", "config_path"),
];

/// The key a name of the desktop app stands for; `None` for any other name.
fn renamed(name: &str) -> Option<&'static str> {
    RENAMED
        .iter()
        .find(|(old, _)| *old == name)
        .map(|(_, new)| *new)
}

/// Refuses a name of the desktop app, alone or after a profile (`work.dbPath`).
///
/// # Errors
/// A usage error that names the key's own name.
pub fn refuse_renamed(key: &str) -> AppResult<()> {
    let bare = key.rsplit_once('.').map_or(key, |(_, k)| k);
    match renamed(bare) {
        Some(new) => Err(AppError::usage(format!(
            "there is no key `{bare}`; it is written `{new}`"
        ))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_of_the_desktop_app_is_told_the_key() {
        let err = refuse_renamed("work.dbPath").unwrap_err();
        assert_eq!(err.code, "invalid_arguments");
        assert!(err.message.contains("`db_path`"), "{}", err.message);
        assert!(refuse_renamed("aiInsightAllowContext").is_err());
        for key in [
            "db_path",
            "work.db_path",
            "ai_insight_allow_context",
            "my_own_setting",
        ] {
            assert!(refuse_renamed(key).is_ok(), "{key}");
        }
    }

    #[test]
    fn every_name_is_one_of_the_desktop_app_and_every_key_is_snake_case() {
        for (old, new) in RENAMED {
            assert!(old.chars().any(|c| c.is_ascii_uppercase()), "{old}");
            assert!(
                new.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{new}"
            );
        }
    }
}
