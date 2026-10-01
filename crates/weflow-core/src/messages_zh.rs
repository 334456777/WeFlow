//! Chinese translations of runtime error messages. `{}` matches any text (see `locale::translate_with`).
//! Messages without an entry are shown in English.

use crate::locale::translate_with;

const CATALOG: &[(&str, &str)] = &[
    // CLI
    ("this removes files; add --yes to confirm", "此操作会删除文件；请加上 --yes 确认"),
    ("auto download failed to start", "自动下载启动失败"),
    ("clearing the caches failed", "清理缓存失败"),
    ("payloads_json must be a JSON array: {}", "payloads_json 必须是 JSON 数组：{}"),
    ("invalid value '{}' for {}; use a whole number of seconds (0 or more)", "{1} 的值 '{0}' 无效；请使用整数秒数（0 或更大）"),
    ("old Electron config not found; pass an explicit path", "未找到旧版 Electron 配置；请显式指定路径"),
    ("pass exactly one of --local-id or --server-id", "--local-id 与 --server-id 必须且只能指定其一"),
    ("failed to create {}: {}", "创建 {} 失败：{}"),
    ("failed to write {}: {}", "写入 {} 失败：{}"),
    ("failed to read {}: {}", "读取 {} 失败：{}"),
    ("create {}: {}", "创建 {} 失败：{}"),
    ("messages_json must be a JSON array: {}", "messages_json 必须是 JSON 数组：{}"),
    ("--display-name must be group-nickname, remark or nickname", "--display-name 必须是 group-nickname、remark 或 nickname"),
    ("unsupported message export format: {}; supported: {}", "不支持的消息导出格式：{}；支持：{}"),
    ("unknown media kind: {}; use image, voice, video, emoji or all", "未知的媒体类型：{}；请使用 image、voice、video、emoji 或 all"),
    ("unsupported media type: {}; use image, voice, video, emoji or all", "不支持的媒体类型：{}；请使用 image、voice、video、emoji 或 all"),
    ("--start ({}) is after --end ({})", "--start（{0}）晚于 --end（{1}）"),
    ("request failed", "请求失败"),
    ("payload_json must be JSON: {}", "payload_json 必须是合法的 JSON：{}"),
    ("serve needs at least one of --http, --message-push, --insight, --image-auto-download", "serve 至少需要 --http、--message-push、--insight、--image-auto-download 之一"),
    ("invalid listen address: {}", "监听地址无效：{}"),
    ("failed to bind HTTP server: {}", "绑定 HTTP 服务失败：{}"),
    ("HTTP server stopped: {}", "HTTP 服务已停止：{}"),
    ("interrupted by user", "已被用户中断"),
    ("failed to serialize JSON: {}", "序列化 JSON 失败：{}"),
    ("unsupported export format: {}; supported: json, csv, txt, html, excel, sql, chatlab, weclone", "不支持的导出格式：{}；支持：json、csv、txt、html、excel、sql、chatlab、weclone"),
    ("CSV export expects an array; use --format json for nested data", "CSV 导出需要数组数据；嵌套数据请使用 --format json"),
    // config
    ("config override must include a parent directory", "配置文件路径必须包含上级目录"),
    ("failed to prepare runtime assets: {}", "准备运行时资源失败：{}"),
    ("failed to locate platform config directory", "无法定位系统配置目录"),
    ("expected scalar string value", "需要字符串值"),
    ("expected integer value", "需要整数值"),
    ("expected boolean value", "需要布尔值"),
    ("missing db_path; pass --db-path or run config set db_path", "缺少 db_path；请传入 --db-path 或运行 config set db_path"),
    ("missing decrypt_key; pass --decrypt-key or run config set decrypt_key", "缺少 decrypt_key；请传入 --decrypt-key 或运行 config set decrypt_key"),
    ("profile not found: {}", "未找到配置档案：{}"),
    ("wxid is not configured", "未配置 wxid"),
    ("no account or database path configured", "未配置账号或数据库路径"),
    ("account directory not found", "未找到账号目录"),
    ("no current account (wxid) configured, nothing to clear", "未配置当前账号（wxid），没有可清理的内容"),
    ("choose at least one thing to clear: --cache and/or --exports-dir <dir>", "请至少选择一项要清理的内容：--cache 和/或 --exports-dir <目录>"),
    // services
    ("failed to create HTTP client: {}", "创建 HTTP 客户端失败：{}"),
    ("failed to download image: {}", "下载图片失败：{}"),
    ("image download failed with status {}", "图片下载失败，状态码 {}"),
    ("failed to read image response: {}", "读取图片响应失败：{}"),
    ("WeChat process not found (looked for Weixin.exe and WeChat.exe); start WeChat first or pass --pid", "未找到微信进程（已查找 Weixin.exe 和 WeChat.exe）；请先启动微信或传入 --pid"),
    ("permission denied: cannot open the WeChat process (pid {}). Run the terminal as administrator, close security software that blocks it, and make sure WeChat itself is not running as administrator. ({})", "权限不足：无法打开微信进程（pid {0}）。请以管理员身份运行终端，关闭拦截的安全软件，并确认微信本身没有以管理员身份运行。（{1}）"),
    ("wx_key library not found; key extraction requires the platform-specific native library", "未找到 wx_key 库；提取密钥需要对应平台的原生库"),
    ("image key not available; configure image_xor_key or use the wx_key native library", "图片密钥不可用；请配置 image_xor_key 或使用 wx_key 原生库"),
    ("failed to parse the image key data", "解析图片密钥数据失败"),
    ("no valid key code found (the kvcomm cache is empty); open a few images in WeChat first", "未找到有效的密钥码（kvcomm 缓存为空）；请先在微信中打开几张图片"),
    ("the cached codes do not match this account's wxid; check --wxid / the account directory, or use `key scan-image`", "缓存的密钥码与该账号的 wxid 不匹配；请检查 --wxid / 账号目录，或使用 `key scan-image`"),
    ("image key scanning requires platform-specific native library", "扫描图片密钥需要对应平台的原生库"),
    ("no messages found for this session in the given range", "指定范围内未找到该会话的消息"),
    ("{} is not yet ported to the native Rust CLI", "{} 尚未移植到原生 Rust CLI"),
    ("no message sessions found", "未找到消息会话"),
    ("session table error: {}", "会话表错误：{}"),
    ("invalid CDN URL", "CDN 地址无效"),
    ("download failed", "下载失败"),
    ("message not found", "未找到该消息"),
    ("session id cannot be empty", "会话 ID 不能为空"),
    ("invalid group chat id", "群聊 ID 无效"),
    ("chatroom id must not be empty", "群聊 ID 不能为空"),
    ("member id must not be empty", "成员 ID 不能为空"),
    ("group aggregation failed", "群聊统计汇总失败"),
    ("the group has no members", "该群没有成员"),
    ("invalid message id", "消息 ID 无效"),
    ("image has no md5 / datName, cannot locate the original file", "图片没有 md5 / datName，无法定位原文件"),
    ("image decrypt failed", "图片解密失败"),
    ("failed to read {}: {}", "读取 {} 失败：{}"),
    ("base statistics failed: {}", "基础统计失败：{}"),
    ("failed to get dual-report statistics: {}", "获取双人报告统计失败：{}"),
    ("failed to read the Moments timeline", "读取朋友圈时间线失败"),
    ("DLL error {}", "DLL 错误 {}"),
    ("username must not be empty", "用户名不能为空"),
    ("url must not be empty", "url 不能为空"),
    ("HTTP {}", "HTTP {}"),
    ("image decryption failed: unrecognized image format", "图片解密失败：无法识别的图片格式"),
    ("empty image data", "图片数据为空"),
    ("invalid image data (wrong key or corrupt cache)", "图片数据无效（密钥错误或缓存已损坏）"),
    ("failed to download the emoji", "下载表情失败"),
    ("SILK decode failed: {}", "SILK 解码失败：{}"),
    ("message timestamp not found", "未找到消息时间戳"),
    ("voice data not found (play the voice message in WeChat once first)", "未找到语音数据（请先在微信中播放一次该语音消息）"),
];

pub fn translate(message: &str) -> Option<String> {
    translate_with(CATALOG, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_fixed_and_templated_messages() {
        assert_eq!(
            translate("message not found").as_deref(),
            Some("未找到该消息")
        );
        assert_eq!(
            translate("failed to write out.txt: denied").as_deref(),
            Some("写入 out.txt 失败：denied")
        );
        assert_eq!(
            translate("--start (b) is after --end (a)").as_deref(),
            Some("--start（b）晚于 --end（a）")
        );
        assert_eq!(translate("not in the catalog"), None);
    }
}
