//! Which loaded module `xkey_helper` scans, and the version switch that picks
//! `KNOWN_RVA`.
//!
//! WeChat 4.1.8 and later keep the main code in `wechat.dylib`; older clients
//! put it in `/Contents/MacOS/WeChat`.  Either module is only accepted above
//! 100 MiB, and finding both is an error rather than a guess.

/// The module size above which a candidate is the WeChat code.
pub(super) const MIN_MODULE_SIZE: u64 = 100 * 1024 * 1024;

/// The offset of the database-key sink in 4.1.7 and earlier.
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

/// The module selection of the original scanner: a `wechat.dylib` over
/// 100 MiB wins, otherwise the main binary; both is ambiguous.
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

/// `KNOWN_RVA` for the version in `Info.plist`, as the original decided it:
/// 4.1.8, 4.1.9, 4.2 and later move the code, so `0` asks for the semantic
/// scan; everything else uses the 4.1.7 offset.
pub(super) fn known_rva(version: Option<&str>) -> u64 {
    let Some(version) = version else {
        return LEGACY_KNOWN_RVA;
    };
    let new_mode = version.starts_with("4.1.8")
        || version.starts_with("4.1.9")
        || version.starts_with("4.2")
        || version
            .as_bytes()
            .first()
            .is_some_and(|first| *first > b'4');
    if new_mode {
        0
    } else {
        LEGACY_KNOWN_RVA
    }
}

/// `/Applications/WeChat.app/Contents/MacOS/WeChat` ->
/// `/Applications/WeChat.app/Contents/Info.plist`.
pub(super) fn info_plist_path(executable: &str) -> Option<String> {
    let end = executable
        .find(".app/")
        .or_else(|| executable.find(".app\\"))?
        + ".app".len();
    Some(format!("{}/Contents/Info.plist", &executable[..end]))
}

/// `CFBundleShortVersionString` from the bundle's `Info.plist`.
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
    fn switches_on_the_versions_the_original_switched_on() {
        for version in ["4.1.8", "4.1.9.1", "4.2", "4.2.0.1", "5.0.1"] {
            assert_eq!(known_rva(Some(version)), 0, "{version}");
        }
        for version in ["4.1.7", "4.1.10", "4.3", "3.9.2", ""] {
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
