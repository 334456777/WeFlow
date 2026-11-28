//! `xkey_helper` 扫描哪个已加载的模块，以及按版本选择 `KNOWN_RVA`。
//!
//! 微信 4.1.8 及以后的主要代码在 `wechat.dylib` 中，更早的版本在 `/Contents/MacOS/WeChat` 中。两种模块都只在超过
//! 100 MiB 时才采用；两种都找到时报错，不去猜测用哪一个。

/// 模块超过这个大小才被当作微信的代码。
pub(super) const MIN_MODULE_SIZE: u64 = 100 * 1024 * 1024;

/// 4.1.7 及更早版本中数据库密钥 sink 的偏移。
pub(super) const LEGACY_KNOWN_RVA: u64 = 0x458c570;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Module {
    pub path: String,
    pub base: u64,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    WechatDylib(Module),
    Main(Module),
    Ambiguous,
    None,
}

fn lower(path: &str) -> String {
    path.to_ascii_lowercase()
}

pub(super) fn is_wechat_dylib(path: &str) -> bool {
    lower(path).contains("wechat.dylib")
}

pub(super) fn is_main_binary(path: &str) -> bool {
    let lower = lower(path);
    if lower.contains("wechatappex")
        || lower.contains("/helpers/")
        || lower.contains("/plugins/")
        || lower.contains("crashpad")
        || lower.contains("helper")
    {
        return false;
    }
    lower.ends_with("/contents/macos/wechat") || lower.ends_with("/wechat")
}

/// 与桌面版扫描器相同的模块选择：优先选超过 100 MiB 的 `wechat.dylib`，否则选主二进制；两者都有时无法确定。
pub(super) fn select(modules: &[Module]) -> Choice {
    let dylib = modules
        .iter()
        .find(|module| is_wechat_dylib(&module.path) && module.size > MIN_MODULE_SIZE)
        .cloned();
    let main = modules
        .iter()
        .find(|module| is_main_binary(&module.path) && module.size > MIN_MODULE_SIZE)
        .cloned();
    match (dylib, main) {
        (Some(_), Some(_)) => Choice::Ambiguous,
        (Some(module), None) => Choice::WechatDylib(module),
        (None, Some(module)) => Choice::Main(module),
        (None, None) => Choice::None,
    }
}

/// 代码仍在 [`LEGACY_KNOWN_RVA`] 处的最新版本。
const LEGACY_LAST_VERSION: [u64; 3] = [4, 1, 7];

/// 版本字符串的前三个数字部分，缺少的部分按 0 算，之后的部分忽略。
fn version_key(version: &str) -> Option<[u64; 3]> {
    let mut key = [0_u64; 3];
    let mut parts = version.trim().split('.');
    for slot in &mut key {
        let Some(part) = parts.next() else {
            return Some(key);
        };
        let digits = part
            .as_bytes()
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        *slot = part[..digits].parse().ok()?;
    }
    Some(key)
}

/// `Info.plist` 中的版本对应的 `KNOWN_RVA`：4.1.8 及以后的代码换了位置，取 `0` 表示按语义扫描；4.1.7 及更早的用
/// 4.1.7 的偏移。`4.1.7.31` 这样带构建号的版本仍算作 4.1.7。
pub(super) fn known_rva(version: Option<&str>) -> u64 {
    match version.and_then(version_key) {
        Some(key) if key > LEGACY_LAST_VERSION => 0,
        _ => LEGACY_KNOWN_RVA,
    }
}

/// 由可执行文件的路径得到 `Info.plist` 的路径，如 `/Applications/WeChat.app/Contents/MacOS/WeChat` ->
/// `/Applications/WeChat.app/Contents/Info.plist`。
pub(super) fn info_plist_path(executable: &str) -> Option<String> {
    let end = executable
        .find(".app/")
        .or_else(|| executable.find(".app\\"))?
        + ".app".len();
    Some(format!("{}/Contents/Info.plist", &executable[..end]))
}

/// 从应用包的 `Info.plist` 中读出 `CFBundleShortVersionString`。
pub(super) fn read_version(executable: &str) -> Option<String> {
    let path = info_plist_path(executable)?;
    let plist = std::fs::read_to_string(path).ok()?;
    let regex =
        regex::Regex::new(r"<key>CFBundleShortVersionString</key>\s*<string>([^<]+)</string>")
            .ok()?;
    let captures = regex.captures(&plist)?;
    Some(captures.get(1)?.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(path: &str, size: u64) -> Module {
        Module {
            path: path.to_string(),
            base: 0x1_0000_0000,
            size,
        }
    }

    #[test]
    fn reads_the_version_information_the_original_read() {
        let path = "/Applications/WeChat.app/Contents/MacOS/WeChat";
        assert_eq!(
            info_plist_path(path).as_deref(),
            Some("/Applications/WeChat.app/Contents/Info.plist")
        );
        assert_eq!(info_plist_path("/tmp/WeChat"), None);
    }

    #[test]
    fn uses_the_semantic_scan_from_4_1_8() {
        for version in [
            "4.1.8", "4.1.8.53", "4.1.9.1", "4.1.10", "4.2", "4.2.0.1", "4.3", "4.3.0.1", "4.10",
            "5.0.1",
        ] {
            assert_eq!(known_rva(Some(version)), 0, "{version}");
        }
        for version in [
            "4.1.7", "4.1.7.31", "4.1.6", "4.0.5", "4.1", "3.9.2", "", "unknown",
        ] {
            assert_eq!(known_rva(Some(version)), LEGACY_KNOWN_RVA, "{version}");
        }
        assert_eq!(known_rva(None), LEGACY_KNOWN_RVA);
    }

    #[test]
    fn reads_the_version_from_the_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("WeChat.app");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            "<plist><dict><key>CFBundleShortVersionString</key>\n<string>4.1.8.53</string></dict></plist>",
        )
        .unwrap();
        let executable = app.join("Contents/MacOS/WeChat");
        assert_eq!(
            read_version(executable.to_str().unwrap()).as_deref(),
            Some("4.1.8.53")
        );
    }

    #[test]
    fn selects_the_module_the_original_selected() {
        let small = module("/Applications/WeChat.app/Contents/MacOS/wechat.dylib", 42);
        let dylib = module(
            "/Applications/WeChat.app/Contents/Frameworks/wechat.dylib",
            120 * 1024 * 1024,
        );
        let main = module(
            "/Applications/WeChat.app/Contents/MacOS/WeChat",
            150 * 1024 * 1024,
        );
        assert_eq!(
            select(&[small, dylib.clone()]),
            Choice::WechatDylib(dylib.clone())
        );
        assert_eq!(
            select(std::slice::from_ref(&main)),
            Choice::Main(main.clone())
        );
        assert_eq!(select(&[dylib, main]), Choice::Ambiguous);
        assert_eq!(select(&[]), Choice::None);
    }

    #[test]
    fn ignores_helpers_plugins_and_appex() {
        assert!(!is_main_binary(
            "/Applications/WeChat.app/Contents/MacOS/WeChatAppEx"
        ));
        assert!(!is_main_binary(
            "/Applications/WeChat.app/Contents/Frameworks/WeChatHelper.app/Contents/MacOS/WeChatHelper"
        ));
        assert!(!is_main_binary(
            "/Applications/WeChat.app/Contents/PlugIns/WeChat"
        ));
        assert!(is_main_binary(
            "/Applications/WeChat.app/Contents/MacOS/WeChat"
        ));
    }
}
