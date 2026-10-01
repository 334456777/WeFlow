//! Locale-aware string ordering, the Rust counterpart of `Intl.Collator('zh-CN')` and `String.localeCompare`.
//!
//! The desktop app sorts contact and member names with the ICU collator (Chinese names by pinyin, Latin
//! letters case-insensitively before the first difference in case). The same ICU collation data is used here
//! so lists come out in the order the desktop app shows them.

use std::cmp::Ordering;
use std::sync::OnceLock;

use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use icu_locale_core::locale;

fn build(prefs: CollatorPreferences) -> CollatorBorrowed<'static> {
    Collator::try_new(prefs, CollatorOptions::default())
        .expect("compiled collation data is always available")
}

/// `new Intl.Collator('zh-CN').compare(a, b)`: pinyin order for Chinese; digits first, then Han (CLDR puts Han before Latin for zh), then Latin letters.
pub fn compare_zh(a: &str, b: &str) -> Ordering {
    static ZH: OnceLock<CollatorBorrowed<'static>> = OnceLock::new();
    ZH.get_or_init(|| build(CollatorPreferences::from(&locale!("zh-CN"))))
        .compare(a, b)
}

/// `a.localeCompare(b)` without a locale: the root (English-like) collation.
pub fn compare_default(a: &str, b: &str) -> Ordering {
    static ROOT: OnceLock<CollatorBorrowed<'static>> = OnceLock::new();
    ROOT.get_or_init(|| build(CollatorPreferences::from(&locale!("en-US"))))
        .compare(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut v: Vec<&'static str>, f: fn(&str, &str) -> Ordering) -> Vec<&'static str> {
        v.sort_by(|a, b| f(a, b));
        v
    }

    #[test]
    fn chinese_names_sort_by_pinyin_between_digits_and_latin() {
        let names = vec![
            "张三", "李四", "王五", "Alice", "bob", "123", "陈明", "阿里", "赵六",
        ];
        assert_eq!(
            sorted(names, compare_zh),
            ["123", "阿里", "陈明", "李四", "王五", "张三", "赵六", "Alice", "bob"]
        );
    }

    #[test]
    fn same_pinyin_is_ordered_by_tone_and_then_by_character() {
        // 马 (mǎ), 妈 (mā), 麻 (má): first tone, second tone, third tone
        assert_eq!(
            sorted(vec!["马", "妈", "麻"], compare_zh),
            ["妈", "麻", "马"]
        );
    }

    #[test]
    fn latin_ordering_ignores_case_first_and_punctuation_comes_first() {
        assert_eq!(
            sorted(vec!["b", "B", "a", "A", "_x", "1"], compare_default),
            ["_x", "1", "a", "A", "b", "B"]
        );
        assert_eq!(compare_default("wxid_a", "wxid_a"), Ordering::Equal);
        assert_eq!(compare_zh("wxid_a", "wxid_b"), Ordering::Less);
    }
}
