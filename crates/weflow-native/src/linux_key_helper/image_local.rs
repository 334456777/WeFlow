//! The `image_local` mode: derive image keys from `kvcomm` filenames.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::process;

const ACCOUNT_ROOTS: [&str; 6] = [
    "Documents/xwechat_files",
    "Documents/WeChat_Data",
    "Documents/WeChat_Data/xwechat_files",
    ".local/share/WeChat/xwechat_files",
    ".xwechat/xwechat_files",
    "xwechat_files",
];

const KVCOMM_ROOTS: [&str; 4] = [
    ".xwechat/net/kvcomm",
    "Documents/WeChat_Data/home/.xwechat/net/kvcomm",
    // The original helper lists this path twice; codes are deduplicated below.
    ".xwechat/net/kvcomm",
    "Documents/.xwechat/net/kvcomm",
];

fn root(home: Option<&Path>, relative: &str) -> PathBuf {
    match home {
        Some(home) => home.join(relative),
        None => PathBuf::from("/").join(relative),
    }
}

/// `cleanWxid`: keep everything before the second underscore.
fn clean_wxid(name: &str) -> &str {
    let Some(first) = name.find('_') else {
        return name;
    };
    match name[first + 1..].find('_') {
        Some(second) => &name[..first + 1 + second],
        None => name,
    }
}

fn accounts(home: Option<&Path>) -> Vec<String> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for relative in ACCOUNT_ROOTS {
        let entries = match std::fs::read_dir(root(home, relative)) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with('.') || !name.starts_with("wxid_") {
                continue;
            }
            let is_directory = entry
                .metadata()
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false);
            if !is_directory {
                continue;
            }
            let wxid = clean_wxid(name).to_string();
            if seen.insert(wxid.clone()) {
                found.push(wxid);
            }
        }
    }
    if found.is_empty() {
        found.push("unknown".to_string());
    }
    found
}

/// `key_(?:[a-zA-Z]+_)?(\d+)_.+`, with the code limited to 1..=u32::MAX.
fn code_of(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("key_")?;
    let rest = if rest.starts_with(|character: char| character.is_ascii_alphabetic()) {
        let underscore = rest.find('_')?;
        if !rest[..underscore]
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic())
        {
            return None;
        }
        &rest[underscore + 1..]
    } else {
        rest
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let code: u64 = rest[..digits].parse().ok()?;
    let tail = &rest[digits..];
    if !tail.starts_with('_') || tail.len() < 2 {
        return None;
    }
    u32::try_from(code).ok().filter(|code| *code != 0)
}

fn codes(home: Option<&Path>) -> Vec<u32> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for relative in KVCOMM_ROOTS {
        let entries = match std::fs::read_dir(root(home, relative)) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let is_file = entry
                .metadata()
                .map(|metadata| metadata.is_file())
                .unwrap_or(false);
            if !is_file {
                continue;
            }
            if let Some(code) = code_of(name) {
                if seen.insert(code) {
                    found.push(code);
                }
            }
        }
    }
    found
}

fn md5_prefix(code: u32, wxid: &str) -> String {
    use md5::{Digest, Md5};
    let digest = Md5::digest(format!("{code}{wxid}").as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The `data` object of the helper's `image_local` answer.
pub fn image_local_at(home: Option<&Path>) -> Result<String, &'static str> {
    let accounts = accounts(home);
    let codes = codes(home);
    if codes.is_empty() {
        return Err("ERROR:KVCOMM_NOT_FOUND");
    }
    let mut output = String::from("{\"accounts\":[");
    for (account_index, wxid) in accounts.iter().enumerate() {
        if account_index != 0 {
            output.push(',');
        }
        output.push_str("{\"wxid\":\"");
        output.push_str(wxid);
        output.push_str("\",\"keys\":[");
        for (code_index, code) in codes.iter().enumerate() {
            if code_index != 0 {
                output.push(',');
            }
            let xor = code & 0xff;
            output.push_str(&format!(
                "{{\"code\":{code},\"xorKey\":{xor},\"aesKey\":\"{}\"}}",
                md5_prefix(*code, wxid)
            ));
        }
        output.push_str("]}");
    }
    output.push_str("]}");
    Ok(output)
}

pub fn image_local() -> Result<String, &'static str> {
    let home = process::home_dir();
    image_local_at(home.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kvcomm_names_like_the_helper() {
        for (name, expected) in [
            ("key_123_abc", Some(123)),
            ("key_abc_123_def", Some(123)),
            ("key_123_a_b", Some(123)),
            ("key_123_abc.statistic", Some(123)),
            ("key_0123_abc", Some(123)),
            ("key_4294967295_abc", Some(u32::MAX)),
            ("key_123_.x", Some(123)),
            ("key_123_ABC", Some(123)),
            ("key_1_x", Some(1)),
            ("key_123", None),
            ("key_123_", None),
            ("key_1234", None),
            ("key_0_abc", None),
            ("key_4294967296_abc", None),
            ("key__123_abc", None),
            ("key_abc123_456_def", None),
            ("prefix_key_123_abc", None),
            ("xkey_123_abc", None),
            ("key_1_", None),
        ] {
            assert_eq!(code_of(name), expected, "{name}");
        }
    }

    #[test]
    fn cleans_wxid_before_the_second_underscore() {
        assert_eq!(clean_wxid("wxid_abc123"), "wxid_abc123");
        assert_eq!(clean_wxid("wxid_abc123_1234"), "wxid_abc123");
        assert_eq!(clean_wxid("wxid_"), "wxid_");
        assert_eq!(clean_wxid("unknown"), "unknown");
    }

    #[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
    mod image_local_output {
        use super::*;
        use crate::fixture::TestDir;

        fn parse(home: &Path) -> serde_json::Value {
            serde_json::from_str(&image_local_at(Some(home)).unwrap()).unwrap()
        }

        #[test]
        fn derives_the_keys_of_every_account_found() {
            let home = TestDir::new("weflow-image-local");
            std::fs::create_dir_all(home.join("Documents/xwechat_files/wxid_abc123_1234")).unwrap();
            std::fs::create_dir_all(home.join("Documents/WeChat_Data/wxid_def456")).unwrap();
            std::fs::create_dir_all(home.join(".xwechat/xwechat_files/wxid_ghi789")).unwrap();
            let kvcomm = home.join(".xwechat/net/kvcomm");
            std::fs::create_dir_all(&kvcomm).unwrap();
            for name in ["key_111_a", "key_222_b", "key_abc_333_c", "not_a_key"] {
                std::fs::File::create(kvcomm.join(name)).unwrap();
            }
            let value = parse(home.path());
            let accounts = value["accounts"].as_array().unwrap();
            let wxids: Vec<&str> = accounts
                .iter()
                .map(|account| account["wxid"].as_str().unwrap())
                .collect();
            assert_eq!(wxids, ["wxid_abc123", "wxid_def456", "wxid_ghi789"]);
            for account in accounts {
                let wxid = account["wxid"].as_str().unwrap();
                let keys = account["keys"].as_array().unwrap();
                let mut codes: Vec<u32> = keys
                    .iter()
                    .map(|key| u32::try_from(key["code"].as_u64().unwrap()).unwrap())
                    .collect();
                codes.sort_unstable();
                assert_eq!(codes, [111, 222, 333]);
                for key in keys {
                    let code = u32::try_from(key["code"].as_u64().unwrap()).unwrap();
                    assert_eq!(key["xorKey"], code & 0xff);
                    assert_eq!(key["aesKey"], md5_prefix(code, wxid));
                }
            }
        }

        #[test]
        fn names_the_account_unknown_when_no_account_directory_exists() {
            let home = TestDir::new("weflow-image-local-unknown");
            let kvcomm = home.join(".xwechat/net/kvcomm");
            std::fs::create_dir_all(&kvcomm).unwrap();
            std::fs::File::create(kvcomm.join("key_42_x")).unwrap();
            let value = parse(home.path());
            let accounts = value["accounts"].as_array().unwrap();
            assert_eq!(accounts.len(), 1);
            assert_eq!(accounts[0]["wxid"], "unknown");
            assert_eq!(accounts[0]["keys"][0]["aesKey"], md5_prefix(42, "unknown"));
        }

        #[test]
        fn reports_a_missing_kvcomm_code() {
            let home = TestDir::new("weflow-image-local-empty");
            std::fs::create_dir_all(home.join("Documents/xwechat_files/wxid_abc")).unwrap();
            assert_eq!(
                image_local_at(Some(home.path())),
                Err("ERROR:KVCOMM_NOT_FOUND")
            );
        }
    }
}
