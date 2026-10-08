//! Chinese translations of runtime error messages. `{}` matches any text (see `locale::translate_with`).
//! Messages without an entry are shown in English.

use crate::locale::translate_with;

const CATALOG: &[(&str, &str)] = &[
    ("Rust image-key acquisition needs kvcomm directories (none found; pass --kvcomm-dir) and a nonzero scan budget", "Rust 图片取钥需要 kvcomm 目录（未找到，请用 --kvcomm-dir 指定）和非零扫描预算"),
    ("--kvcomm-dir and --scan-budget require --method rust", "--kvcomm-dir 和 --scan-budget 需配合 --method rust 使用"),
    ("No kvcomm directory could be read", "所有 kvcomm 目录均无法读取"),
    ("No candidate code found in the kvcomm filenames", "kvcomm 文件名中未找到候选 code"),
    ("No valid V2 thumbnail found in the selected account", "选定账号中未找到有效 V2 缩略图模板"),
    ("The cached codes do not match the selected account's V2 samples", "缓存 code 与选定账号的 V2 样本不匹配"),
    ("Multiple distinct image-key pairs match the selected account; no pair was chosen", "多个不同图片密钥组合匹配选定账号，未选择任何组合"),
    // CLI
    ("this removes files; add --yes to confirm", "此操作会删除文件；请加上 --yes 确认"),
    ("choose what to clear: {} (`weflow cache list` shows each part)", "请选择要清理的部分：{}（`weflow cache list` 可查看各部分）"),
    ("failed to read the answer: {}", "读取输入失败：{}"),
    ("auto download failed to start", "自动下载启动失败"),
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
    ("invalid value '{}' for lang; use en or zh", "lang 的值 '{}' 无效；请使用 en 或 zh"),
    ("config override must include a parent directory", "配置文件路径必须包含上级目录"),
    ("failed to prepare runtime assets: {}", "准备运行时资源失败：{}"),
    ("failed to locate platform config directory", "无法定位系统配置目录"),
    ("expected scalar string value", "需要字符串值"),
    ("expected integer value", "需要整数值"),
    ("expected boolean value", "需要布尔值"),
    ("missing db_path; run config set db_path", "缺少 db_path；请运行 config set db_path"),
    ("missing decrypt_key; run config set decrypt_key", "缺少 decrypt_key；请运行 config set decrypt_key"),
    ("profile not found: {}", "未找到配置档案：{}"),
    ("wxid is not configured", "未配置 wxid"),
    ("no account or database path configured", "未配置账号或数据库路径"),
    ("account directory not found", "未找到账号目录"),
    ("no current account (wxid) configured, nothing to clear", "未配置当前账号（wxid），没有可清理的内容"),
    ("nothing chosen to clear: neither the caches nor an export folder", "没有选择要清理的内容：既没有缓存，也没有导出文件夹"),
    // services
    ("failed to create HTTP client: {}", "创建 HTTP 客户端失败：{}"),
    ("failed to download image: {}", "下载图片失败：{}"),
    ("image download failed with status {}", "图片下载失败，状态码 {}"),
    ("failed to read image response: {}", "读取图片响应失败：{}"),
    ("timed out after {} s without getting the key; quit WeChat completely, reopen it and click \"Enter WeChat\" in the login window, then run the command again", "等待超时（{} 秒），没有获取到密钥。请确认已完全退出微信后重新打开，并在登录窗口点击「进入微信」，然后重新运行。"),
    ("no WeChat data directory found; pass the directory or run config set db_path", "未找到微信数据目录；请指定目录或运行 config set db_path"),
    ("no WeChat account directory found in {}", "在 {} 中没有找到微信账号目录"),
    ("WeChat process not found (looked for Weixin.exe and WeChat.exe); start WeChat first or pass --pid", "未找到微信进程（已查找 Weixin.exe 和 WeChat.exe）；请先启动微信或传入 --pid"),
    ("permission denied: cannot open the WeChat process (pid {}). Run the terminal as administrator, close security software that blocks it, and make sure WeChat itself is not running as administrator. ({})", "无法打开微信进程（pid {0}）。请以管理员身份运行终端，关闭可能拦截的安全软件，并确认微信本身没有以管理员身份运行。（{1}）"),
    ("wx_key library not found; key extraction requires the platform-specific native library", "未找到 wx_key 库；提取密钥需要对应平台的原生库"),
    ("image key not available; configure image_xor_key or use the wx_key native library", "图片密钥不可用；请配置 image_xor_key 或使用 wx_key 原生库"),
    ("failed to parse the image key data", "解析图片密钥数据失败"),
    ("no valid key code found (the kvcomm cache is empty); open a few images in WeChat first", "未找到有效的密钥码（kvcomm 缓存为空）；请先在微信中打开几张图片"),
    ("the cached codes do not match this account's wxid; check the configured wxid / the account directory, or use `key scan-image`", "缓存的密钥码与该账号的 wxid 不匹配；请检查已配置的 wxid / 账号目录，或使用 `key scan-image`"),
    ("image key scanning requires platform-specific native library", "扫描图片密钥需要对应平台的原生库"),
    ("no messages found for this session in the given range", "指定范围内未找到该会话的消息"),
    ("no messages from {} in this session in the given range (--sender takes the bare wxid, as `chat contacts` lists it)", "指定范围内该会话没有 {} 发送的消息（--sender 只接受裸 wxid，即 `chat contacts` 列出的形式）"),
    ("--sender takes the bare wxid: use {}, not the account folder name {}", "--sender 只接受裸 wxid：请使用 {}，而不是账号文件夹名 {}"),
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
    // progress
    ("downloading emojis", "正在下载表情"),
    ("creating backup", "正在创建备份"),
    ("restoring backup", "正在恢复备份"),
    ("connecting…", "正在连接…"),
    ("loading extended statistics…", "正在加载扩展统计…"),
    ("analysing chats…", "正在分析聊天…"),
    ("analysing Moments…", "正在分析朋友圈…"),
    ("collecting contact info…", "正在收集联系人信息…"),
    ("building report…", "正在生成报告…"),
    ("first messages…", "正在读取最早的消息…"),
    ("chat statistics…", "正在统计聊天…"),
    ("done", "完成"),
    ("loading Moments…", "正在加载朋友圈…"),
    ("downloading media", "正在下载媒体"),
    ("downloading avatars", "正在下载头像"),
    ("building ArkmeJSON", "正在生成 ArkmeJSON"),
    ("export finished", "导出完成"),
    ("aggregating…", "正在汇总…"),
    ("copying media", "正在复制媒体"),
    ("downloading ffmpeg", "正在下载 ffmpeg"),
    ("the ffmpeg download address must start with http:// or https://: {}", "ffmpeg 下载地址必须以 http:// 或 https:// 开头：{}"),
    ("WXGF image needs ffmpeg: not found on PATH or installed; run `weflow ffmpeg install` or set FFMPEG_PATH", "WXGF 图片需要 ffmpeg：在 PATH 中和已安装的位置都没有找到；请运行 `weflow ffmpeg install` 或设置 FFMPEG_PATH"),
    ("WXGF image needs ffmpeg: FFMPEG_PATH ({}) cannot be started", "WXGF 图片需要 ffmpeg：FFMPEG_PATH（{}）无法启动"),
    ("WXGF image sent {} needs ffmpeg: not found on PATH or installed; run `weflow ffmpeg install` or set FFMPEG_PATH", "{} 发送的 WXGF 图片需要 ffmpeg：在 PATH 中和已安装的位置都没有找到；请运行 `weflow ffmpeg install` 或设置 FFMPEG_PATH"),
    ("WXGF image sent {} needs ffmpeg: FFMPEG_PATH ({}) cannot be started", "{0} 发送的 WXGF 图片需要 ffmpeg：FFMPEG_PATH（{1}）无法启动"),
    // the longer template first: `{}` would also match the list of the shorter one
    ("{} WXGF images were not exported because ffmpeg was not found (sent {} and {} more, all listed in ffmpegMissingImages); run `weflow ffmpeg install` or set FFMPEG_PATH, then export again", "有 {0} 张 WXGF 图片因为找不到 ffmpeg 没有导出（发送于 {1}，另有 {2} 张，完整列表见 ffmpegMissingImages）；请运行 `weflow ffmpeg install` 或设置 FFMPEG_PATH，然后重新导出"),
    ("{} WXGF images were not exported because ffmpeg was not found (sent {}); run `weflow ffmpeg install` or set FFMPEG_PATH, then export again", "有 {0} 张 WXGF 图片因为找不到 ffmpeg 没有导出（发送于 {1}）；请运行 `weflow ffmpeg install` 或设置 FFMPEG_PATH，然后重新导出"),
    ("SHA-256 of {} does not match: expected {}, got {}", "{} 的 SHA-256 不匹配：应为 {}，实际为 {}"),
    ("download of {} failed: {}", "下载 {} 失败：{}"),
    ("cannot unpack {}: {}", "无法解压 {}：{}"),
    ("no ffmpeg build of {} for {}", "{} 没有适用于 {} 的 ffmpeg 构建"),
    ("ffmpeg was installed at {} but does not start: {}", "ffmpeg 已安装到 {}，但无法启动：{}"),
    ("scanning messages", "正在扫描消息"),
    ("exporting {} media", "正在导出{}媒体"),
    ("reading messages", "正在读取消息"),
    // result fields and hints
    ("no insight was generated (AI not configured, request failed, or the model answered SKIP)", "未生成洞察（AI 未配置、请求失败，或模型回答了 SKIP）"),
    ("waiting for WeChat to start", "正在等待微信启动"),
    ("core component initialization failed", "核心组件初始化失败"),
    ("hook failed", "挂钩失败"),
    ("from stored config; use key scan-image for live extraction", "来自已保存的配置；如需实时提取请使用 key scan-image"),
    ("missing = media messages whose file is not on disk (not downloaded in WeChat) or could not be resolved; stickers need network access. thumbOnly = exported images that are only the thumbnail (open the original in WeChat, then export again for the HD image)", "missing = 文件不在磁盘上（微信中未下载）或无法解析的媒体消息；表情需要联网。thumbOnly = 导出的图片只有缩略图（请先在微信中打开原图，再重新导出以获得高清图）"),
    ("missing image identifier", "缺少图片标识"),
    ("cached image not found", "未找到缓存的图片"),
    ("image file not found, open the image in WeChat and retry", "未找到图片文件，请先在微信中打开该图片后重试"),
    ("image file not found", "未找到图片文件"),
    ("image decrypt key is not configured", "未配置图片解密密钥"),
    ("native decrypt failed, check the image keys: {}", "原生解密失败，请检查图片密钥：{}"),
    ("image cache", "图片缓存"),
    ("unknown error", "未知错误"),
    ("record id is empty", "记录 ID 为空"),
    ("import_table_snapshot is not supported: the native database backend opens WeChat's databases read-only", "不支持 import_table_snapshot：原生数据库后端以只读方式打开微信数据库"),
    ("import_table_snapshot_with_schema is not supported: the native database backend opens WeChat's databases read-only", "不支持 import_table_snapshot_with_schema：原生数据库后端以只读方式打开微信数据库"),
    ("API call failed ({}): {}", "API 调用失败（{0}）：{1}"),
    ("Telegram push failed (chatId={}): {}", "Telegram 推送失败（chatId={0}）：{1}"),
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

    #[test]
    fn the_dates_of_the_images_missing_ffmpeg_stay_in_the_hint() {
        let tail = "; run `weflow ffmpeg install` or set FFMPEG_PATH, then export again";
        assert_eq!(
            translate(&format!("2 WXGF images were not exported because ffmpeg was not found (sent 2023-11-15 06:13:20, 2024-01-02 03:04:05){tail}")).as_deref(),
            Some("有 2 张 WXGF 图片因为找不到 ffmpeg 没有导出（发送于 2023-11-15 06:13:20, 2024-01-02 03:04:05）；请运行 `weflow ffmpeg install` 或设置 FFMPEG_PATH，然后重新导出")
        );
        assert_eq!(
            translate(&format!("12 WXGF images were not exported because ffmpeg was not found (sent 2023-11-15 06:13:20, ? and 2 more, all listed in ffmpegMissingImages){tail}")).as_deref(),
            Some("有 12 张 WXGF 图片因为找不到 ffmpeg 没有导出（发送于 2023-11-15 06:13:20, ?，另有 2 张，完整列表见 ffmpegMissingImages）；请运行 `weflow ffmpeg install` 或设置 FFMPEG_PATH，然后重新导出")
        );
        assert_eq!(
            translate("WXGF image sent 2023-11-15 06:13:20 needs ffmpeg: FFMPEG_PATH (/x/ffmpeg) cannot be started").as_deref(),
            Some("2023-11-15 06:13:20 发送的 WXGF 图片需要 ffmpeg：FFMPEG_PATH（/x/ffmpeg）无法启动")
        );
    }
}
