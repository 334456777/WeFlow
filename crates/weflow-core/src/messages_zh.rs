//! Chinese translations of runtime error messages. `{}` matches any text (see `locale::translate_with`).
//! Messages without an entry are shown in English.

use crate::locale::translate_with;

const CATALOG: &[(&str, &str)] = &[
    ("image-key acquisition needs kvcomm directories and a nonzero scan budget\nNo kvcomm directory was found: pass --kvcomm-dir, and keep --scan-budget above 0.", "图片取钥需要 kvcomm 目录和非零的扫描预算\n未找到 kvcomm 目录：请用 --kvcomm-dir 指定，并让 --scan-budget 大于 0。"),
    ("`key processes` is only available on macOS", "`key processes` 仅支持 macOS"),
    ("`key image-mem` is only available on Linux x64 and macOS", "`key image-mem` 仅支持 Linux x64 和 macOS"),
    ("No kvcomm directory could be read", "所有 kvcomm 目录均无法读取"),
    ("No candidate code found in the kvcomm filenames", "kvcomm 文件名中未找到候选 code"),
    ("No valid V2 thumbnail found in the selected account", "选定账号中未找到有效 V2 缩略图模板"),
    ("No valid V2 thumbnail found in the selected account; open a few images in WeChat first", "选定账号中未找到有效 V2 缩略图模板；请先在微信里打开几张图片"),
    ("The cached codes do not match the selected account's V2 samples", "缓存 code 与选定账号的 V2 样本不匹配"),
    ("Multiple distinct image-key pairs match the selected account; no pair was chosen", "多个不同图片密钥组合匹配选定账号，未选择任何组合"),
    // CLI
    ("this removes files; add --yes to confirm", "此操作会删除文件；请加上 --yes 确认"),
    ("this removes a profile; add --yes to confirm", "此操作会删除档案；请加上 --yes 确认"),
    ("choose what to clear: {}\n`weflow cache list` shows each part.", "请选择要清理的部分：{}\n`weflow cache list` 可查看各部分。"),
    ("failed to read the answer: {}", "读取输入失败：{}"),
    ("payloads_json must be a JSON array: {}", "payloads_json 必须是 JSON 数组：{}"),
    ("invalid value '{}' for {}; use a whole number of seconds, 0 or more", "{1} 的值 '{0}' 无效；请使用不小于 0 的整数秒数"),
    ("pass exactly one of --local-id or --server-id", "--local-id 与 --server-id 必须且只能指定其一"),
    ("local id {} names {} messages in different message databases\nPick one with --create-time; details.candidates lists them.", "本地 ID {} 在不同的消息库中对应 {} 条消息\n请用 --create-time 指定其中一条；details.candidates 列出了这些消息。"),
    ("failed to create {}: {}", "创建 {} 失败：{}"),
    ("failed to write {}: {}", "写入 {} 失败：{}"),
    ("failed to read {}: {}", "读取 {} 失败：{}"),
    ("create {}: {}", "创建 {} 失败：{}"),
    ("messages_json must be a JSON array: {}", "messages_json 必须是 JSON 数组：{}"),
    ("--display-name must be group-nickname, remark or nickname", "--display-name 必须是 group-nickname、remark 或 nickname"),
    ("unsupported message export format: {}; supported: {}", "不支持的消息导出格式：{}；支持：{}"),
    ("unknown media kind: {}; use image, voice, video, emoji, file, avatar or all", "未知的媒体类型：{}；请使用 image、voice、video、emoji、file、avatar 或 all"),
    ("a list takes no {} media; use --media avatar", "列表不附带 {} 媒体；请使用 --media avatar"),
    ("--media avatar puts the avatars on the HTML page; use --format html", "--media avatar 把头像放在 HTML 页面上；请使用 --format html"),
    ("--zip writes a zip: give --out a folder or a .zip file", "--zip 写出的是 zip：--out 请给文件夹或 .zip 文件"),
    ("a backup is a zip: give --out a folder or a .zip file", "备份是 zip 文件：--out 请给文件夹或 .zip 文件"),
    ("move the backup to {}: {}", "把备份移到 {} 失败：{}"),
    ("unsupported media type: {}; use image, voice, video, emoji, file or all", "不支持的媒体类型：{}；请使用 image、voice、video、emoji、file 或 all"),
    ("--start {} is after --end {}", "--start {0} 晚于 --end {1}"),
    ("invalid year {}; use 0 for all years, or 1970-2100", "年份 {} 无效；0 表示所有年份，其余请使用 1970-2100"),
    ("invalid limit {}; use {}-{}", "limit {0} 无效；请使用 {1} 到 {2}"),
    ("payload_json must be JSON: {}", "payload_json 必须是合法的 JSON：{}"),
    ("invalid listen address {}: {}", "监听地址 {0} 无效：{1}"),
    ("failed to bind HTTP server: {}", "绑定 HTTP 服务失败：{}"),
    ("HTTP server stopped: {}", "HTTP 服务已停止：{}"),
    ("interrupted by user", "已被用户中断"),
    ("failed to serialize JSON: {}", "序列化 JSON 失败：{}"),
    ("no contacts of the kinds asked for", "没有所选类型的联系人"),
    ("none of the sessions asked for is in the session list", "会话列表里没有所选的会话"),
    ("unsupported export format: {}; supported: json, csv, txt, html, excel, sql", "不支持的导出格式：{}；支持：json、csv、txt、html、excel、sql"),
    ("unsupported sns export format: {}; supported: json, html, arkmejson", "不支持的朋友圈导出格式：{}；支持：json、html、arkmejson"),
    ("CSV export expects an array; use --format json for nested data", "CSV 导出需要数组数据；嵌套数据请使用 --format json"),
    ("export footprint writes json, csv or excel, not {}", "export footprint 只能写出 json、csv 或 excel，不支持 {}"),
    ("--format {} writes .{} as another format given does; give one of them", "--format {0} 与另一个指定的格式都写出 .{1} 文件；请只指定其中一个"),
    ("several formats are written to a folder: give --out a folder, or pack them with --zip", "多个格式会写进一个文件夹：--out 请给文件夹，或用 --zip 打包"),
    ("cannot serialize the config: {}", "无法序列化配置：{}"),
    ("invalid path {}: {}", "路径 {0} 无效：{1}"),
    ("{} is a folder; give the config file path", "{} 是文件夹；请给出配置文件的路径"),
    ("no decrypt_key to save", "没有可保存的 decrypt_key"),
    ("no image keys to save", "没有可保存的图片密钥"),
    ("`key image` does not know where WeChat keeps its kvcomm cache on this platform; pass --kvcomm-dir", "`key image` 不知道这个平台上微信的 kvcomm 缓存在哪里；请用 --kvcomm-dir 指定"),
    ("cannot locate the weflow executable: {}", "找不到 weflow 可执行文件：{}"),
    ("the command failed: {}", "命令执行失败：{}"),
    ("the media came back empty", "取回的媒体为空"),
    ("url must be an https address of WeChat's Moments media servers", "url 必须是微信朋友圈媒体服务器的 https 地址"),
    ("the media could not be read", "无法读取该媒体"),
    // WebUI
    ("WebUI {} cannot run with this weflow: it needs API {}, this weflow serves API {}", "网页 {} 无法在这个 weflow 上运行：它需要 API {}，这个 weflow 提供 API {}"),
    ("WebUI {} cannot run with this weflow: it needs weflow {} or newer, this is {}", "网页 {} 无法在这个 weflow 上运行：它需要 weflow {} 或更新的版本，当前是 {}"),
    ("WebUI {} cannot run with this weflow: {}", "网页 {} 无法在这个 weflow 上运行：{}"),
    ("it needs API {}, this weflow serves API {}", "它需要 API {}，这个 weflow 提供 API {}"),
    ("it needs weflow {} or newer, this is {}", "它需要 weflow {} 或更新的版本，当前是 {}"),
    ("no published WebUI version suits this weflow", "没有适合这个 weflow 的已发布网页版本"),
    ("--ui-dir {}: no index.html there\nPass the folder of a built WebUI, such as web/dist.", "--ui-dir {}：该目录下没有 index.html\n请指定构建好的网页目录，例如 web/dist。"),
    ("`{}` is not a WebUI version; a version looks like 0.1.0", "`{}` 不是网页的版本号；版本号形如 0.1.0"),
    ("WebUI {} is shown by a running `weflow serve`; stop it first", "网页 {} 正由运行中的 `weflow serve` 显示；请先停止它"),
    ("WebUI {} is shown by a running `weflow serve`; it was kept", "网页 {} 正由运行中的 `weflow serve` 显示，已保留"),
    ("cannot lock WebUI {}: {}", "无法锁定网页 {}：{}"),
    ("cannot wait for the install of WebUI {}: {}", "无法等待网页 {} 安装完成：{}"),
    ("{} is not WebUI metadata: {}", "{} 不是网页的元数据：{}"),
    ("cannot reach the WebUI registry {} for {}: {}", "无法访问网页仓库 {0}，读取 {1} 时出错：{2}"),
    ("not in the WebUI registry: {}", "网页仓库中没有：{}"),
    ("{} in the registry has no page", "仓库中的 {} 没有网页"),
    ("{} in the registry is not a WebUI: its artifact type is {}", "仓库中的 {0} 不是网页：它的制品类型是 {1}"),
    ("the WebUI package of {} is damaged: expected {} bytes with SHA-256 {}, got {} bytes with {}", "网页 {} 的安装包已损坏：应为 {} 字节、SHA-256 为 {}，实际为 {} 字节、SHA-256 为 {}"),
    ("the WebUI package of {} is {} bytes, more than the {} allowed", "网页 {} 的安装包有 {} 字节，超过允许的 {} 字节"),
    ("the WebUI package is refused: {} files, more than {}", "拒绝安装网页安装包：有 {} 个文件，超过 {} 个"),
    ("the WebUI package is refused: `{}` leads out of the folder", "拒绝安装网页安装包：`{}` 指向目录之外"),
    ("the WebUI package is refused: `{}` is a link", "拒绝安装网页安装包：`{}` 是链接"),
    ("the WebUI package is refused: it unpacks to more than {} bytes", "拒绝安装网页安装包：解压后超过 {} 字节"),
    ("the package of {} has no index.html", "{} 的安装包里没有 index.html"),
    ("the package of {} holds {} {}", "{} 的安装包里是 {} {}"),
    ("cannot move the WebUI into {}: {}", "无法把网页移入 {}：{}"),
    ("cannot open WebUI {} in {}", "无法打开 {1} 中的网页 {0}"),
    ("unknown digest {}", "未知的摘要 {}"),
    ("the WebUI package is bigger than its manifest says", "网页安装包比清单中写的大"),
    // config
    ("invalid value '{}' for lang; use en or zh", "lang 的值 '{}' 无效；请使用 en 或 zh"),
    ("invalid value {} for {}; use a whole number from {} to {}", "{1} 的值 {0} 无效；请使用 {2} 到 {3} 之间的整数"),
    ("the config file path must include a parent directory", "配置文件路径必须包含上级目录"),
    ("failed to locate platform config directory", "无法定位系统配置目录"),
    ("expected scalar string value", "需要字符串值"),
    ("expected integer value", "需要整数值"),
    ("invalid value '{}' for {}; use true or false", "{1} 的值 '{0}' 无效；请使用 true 或 false"),
    ("invalid value '{}' for {}; use openai or anthropic", "{1} 的值 '{0}' 无效；请使用 openai 或 anthropic"),
    ("invalid value '{}' for {}; use an IP address such as 127.0.0.1, 0.0.0.0 or [::1]", "{1} 的值 '{0}' 无效；请使用 IP 地址，例如 127.0.0.1、0.0.0.0 或 [::1]"),
    ("failed to update {}: {}", "更新 {} 失败：{}"),
    ("a profile name cannot be empty", "档案名称不能为空"),
    ("invalid profile name '{}'; a name cannot hold a dot, a slash, a backslash or a control character", "档案名称 '{}' 无效；不能包含点、斜杠、反斜杠或控制字符"),
    ("'{}' is reserved for the config file's own settings; choose another profile name", "'{}' 是配置文件自身设置使用的名字；请换一个档案名称"),
    ("'{}' is the current profile; switch to another with `config set current_profile` first", "'{}' 是当前档案；请先用 `config set current_profile` 切换到其他档案"),
    ("there is no profile named '{}'; `config profiles` lists them", "没有名为 '{}' 的档案；`config profiles` 会列出所有档案"),
    ("there is no profile named '{}'", "没有名为 '{}' 的档案"),
    ("there is no profile named '{}'; `config set current_profile {}` makes one", "没有名为 '{0}' 的档案；`config set current_profile {1}` 会建立这个档案"),
    ("there is no profile named '{}'; the setting is written {}", "没有名为 '{0}' 的档案；该设置应写作 {1}"),
    ("there is no profile named '{}'; `config set current_profile {}` makes one, and the setting is written {}", "没有名为 '{0}' 的档案；`config set current_profile {1}` 会建立这个档案，该设置应写作 {2}"),
    ("missing db_path; run config set db_path", "缺少 db_path；请运行 config set db_path"),
    ("missing decrypt_key; run config set decrypt_key", "缺少 decrypt_key；请运行 config set decrypt_key"),
    ("profile not found: {}", "未找到配置档案：{}"),
    ("there is no key `{}`; it is written `{}`", "没有 `{}` 这个键，应写作 `{}`"),
    ("wxid is not configured", "未配置 wxid"),
    ("no account or database path configured", "未配置账号或数据库路径"),
    ("account directory not found", "未找到账号目录"),
    ("no current account configured: wxid is not set, so there is nothing to clear", "未配置当前账号：没有设置 wxid，没有可清理的内容"),
    // services
    ("failed to create HTTP client: {}", "创建 HTTP 客户端失败：{}"),
    ("failed to download image: {}", "下载图片失败：{}"),
    ("image download failed with status {}", "图片下载失败，状态码 {}"),
    ("failed to read image response: {}", "读取图片响应失败：{}"),
    ("timed out after {} s without getting the key\nQuit WeChat completely, reopen it and click \"Enter WeChat\" in the login window, then run the command again.", "等待 {} 秒后超时，没有获取到密钥\n请完全退出微信后重新打开，在登录窗口点击「进入微信」，然后重新运行命令。"),
    ("no WeChat data directory found; pass the directory or run config set db_path", "未找到微信数据目录；请指定目录或运行 config set db_path"),
    ("no WeChat account directory found in {}", "在 {} 中没有找到微信账号目录"),
    ("WeChat process not found\nStart WeChat first, or pass --pid; `weflow key processes` lists WeChat's processes.", "未找到微信进程\n请先启动微信，或传入 --pid；`weflow key processes` 列出微信的进程。"),
    ("WeChat process not found\nStart WeChat and log in, or pass --pid.", "未找到微信进程\n请先启动微信并登录，或传入 --pid。"),
    ("permission denied: cannot attach to WeChat, pid {}\nRun weflow with sudo, and check that macOS lets it debug other processes: System Integrity Protection and code signing decide it.\nReason: {}", "无法附加到微信进程，pid {0}\n请用 sudo 运行 weflow，并检查 macOS 是否允许它调试其他进程：这由系统完整性保护和代码签名决定。\n原因：{1}"),
    ("cannot find where WeChat takes its database key\nMake sure WeChat has fully started and is in front; otherwise this WeChat version is not supported yet. WeChat 4.1.7 and 4.1.8.100 are known to work.\nReason: {}", "找不到微信使用数据库密钥的位置\n请确认微信已完全启动并在前台；否则是这个微信版本尚未适配。已知微信 4.1.7 和 4.1.8.100 可用。\n原因：{}"),
    ("WeChat quit while the command waited for the key\nReopen WeChat, then run the command again.", "等待密钥时微信已退出\n请重新打开微信，再运行命令。"),
    ("cannot hook WeChat\nQuit WeChat completely and restart the Mac before trying again, as more tries can set off macOS's protection.\nReason: {}", "无法挂钩微信\n请彻底退出微信并重启 Mac 后再试，反复尝试可能触发 macOS 的保护机制。\n原因：{}"),
    ("permission denied: cannot open the WeChat process, pid {}\nRun the terminal as administrator, close security software that blocks it, and make sure WeChat itself is not running as administrator.\nReason: {}", "无法打开微信进程，pid {0}\n请以管理员身份运行终端，关闭可能拦截的安全软件，并确认微信本身没有以管理员身份运行。\n原因：{1}"),
    ("permission denied: cannot attach to WeChat, pid {}\nRun weflow as root, or allow ptrace through /proc/sys/kernel/yama/ptrace_scope.\nReason: {}", "无法附加到微信进程，pid {0}\n请以 root 运行 weflow，或通过 /proc/sys/kernel/yama/ptrace_scope 允许 ptrace。\n原因：{1}"),
    ("permission denied: weflow cannot attach to WeChat while /proc/sys/kernel/yama/ptrace_scope is {}\nRun weflow as root, or set ptrace_scope to 0.", "/proc/sys/kernel/yama/ptrace_scope 为 {} 时，weflow 无法附加到微信进程\n请以 root 运行 weflow，或把 ptrace_scope 设为 0。"),
    ("permission denied: /proc/sys/kernel/yama/ptrace_scope is 3, which stops every process from attaching to WeChat, root included, until the system restarts", "/proc/sys/kernel/yama/ptrace_scope 为 3，在系统重启前任何进程都无法附加到微信进程，root 也不例外"),
    ("the ciphertext must be 32 hex characters, one 16-byte block", "密文必须是 32 个十六进制字符，即一个 16 字节的块"),
    ("permission denied: cannot read WeChat's memory, pid {}\nRun weflow with sudo.\nReason: {}", "无法读取微信的内存，pid {0}\n请用 sudo 运行 weflow。\n原因：{1}"),
    ("permission denied: cannot read WeChat's memory, pid {}\nRun weflow as root, or allow ptrace through /proc/sys/kernel/yama/ptrace_scope.\nReason: {}", "无法读取微信的内存，pid {0}\n请以 root 运行 weflow，或通过 /proc/sys/kernel/yama/ptrace_scope 允许 ptrace。\n原因：{1}"),
    ("cannot read WeChat's memory, pid {}: {}", "无法读取微信的内存，pid {0}：{1}"),
    ("a key found with --ciphertext is checked against that block only, so it is not saved; run `key image-mem --save` without --ciphertext to check the key against the account's thumbnails", "用 --ciphertext 找到的密钥只验证了这一块，所以不保存；请不带 --ciphertext 运行 `key image-mem --save`，用账号的缩略图验证密钥"),
    ("no image AES key found in WeChat's memory; open a few images in WeChat, then try again", "在微信的内存中未找到图片 AES 密钥；请在微信里打开几张图片后重试"),
    ("no messages found for this session in the given range", "指定范围内未找到该会话的消息"),
    ("no messages from {} in this session in the given range\n--sender takes the bare wxid, as `chat contacts` lists it.", "指定范围内该会话没有 {} 发送的消息\n--sender 只接受裸 wxid，即 `chat contacts` 列出的形式。"),
    ("--sender takes the bare wxid: use {}, not the account folder name {}", "--sender 只接受裸 wxid：请使用 {}，而不是账号文件夹名 {}"),
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
    ("invalid server id: {}", "服务端消息 ID 无效：{}"),
    ("the message is not an image", "该消息不是图片"),
    ("the message is not a voice message", "该消息不是语音消息"),
    ("the message is not a video", "该消息不是视频"),
    ("the video message has no md5", "该视频消息没有 md5"),
    ("the video is not on this computer: open it in WeChat first", "该视频不在本机：请先在微信中打开该视频"),
    ("the video has no thumbnail or cover", "该视频没有缩略图或封面"),
    ("the HD original of this image is not on this computer: open the image in WeChat and choose View original", "这张图片的高清原图不在本机：请在微信中打开该图片并点击「查看原图」"),
    ("decrypted data is not a valid image", "解密后的数据不是有效的图片"),
    ("failed to save the decrypted emoji", "保存解密后的表情失败"),
    ("`{}` is not a resource cursor; pass the `nextCursor` of a page", "`{}` 不是资源游标；请传入某一页的 `nextCursor`"),
    ("`{}` is not a message cursor; pass the `_cursor` of a message", "`{}` 不是消息游标；请传入某条消息的 `_cursor`"),
    ("the statistics stopped before they were counted", "统计在完成前中止"),
    ("message reader thread panicked", "读取消息的线程异常退出"),
    ("the message readers stopped before the end of the conversation", "读取消息的线程在读完会话前停止"),
    ("the footprint: {}", "无法读取足迹：{}"),
    ("the native {}: {}", "无法读取原生层返回的 {0}：{1}"),
    ("Windows database-key extraction requires x64", "在 Windows 上获取数据库密钥需要 x64"),
    ("Linux database-key extraction requires x64", "在 Linux 上获取数据库密钥需要 x64"),
    ("database-key extraction is not available on this platform", "这个平台不支持获取数据库密钥"),
    ("unknown resource type: {}; use image, video, voice or file", "未知的资源类型：{}；请使用 image、video、voice 或 file"),
    ("unknown media type: {}; use image, video or all", "未知的媒体类型：{}；请使用 image、video 或 all"),
    ("image has no md5 / datName, cannot locate the original file", "图片没有 md5 / datName，无法定位原文件"),
    ("image decrypt failed", "图片解密失败"),
    ("failed to read {}: {}", "读取 {} 失败：{}"),
    ("base statistics failed: {}", "基础统计失败：{}"),
    ("failed to get dual-report statistics: {}", "获取双人报告统计失败：{}"),
    ("username must not be empty", "用户名不能为空"),
    ("url must not be empty", "url 不能为空"),
    ("HTTP {}", "HTTP {}"),
    ("image decryption failed: unrecognized image format", "图片解密失败：无法识别的图片格式"),
    ("empty image data", "图片数据为空"),
    ("invalid image data: wrong key or corrupt cache", "图片数据无效：密钥错误或缓存已损坏"),
    ("failed to download the emoji", "下载表情失败"),
    ("SILK decode failed: {}", "SILK 解码失败：{}"),
    ("message timestamp not found", "未找到消息时间戳"),
    ("voice data not found\nPlay the voice message in WeChat once first.", "未找到语音数据\n请先在微信中播放一次该语音消息。"),
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
    ("WXGF image sent {} cannot be converted because the data is damaged: {}", "{0} 发送的 WXGF 图片无法转换，数据已损坏：{1}"),
    ("WXGF image sent {} cannot be converted because its format is not supported yet: {}", "{0} 发送的 WXGF 图片无法转换，暂不支持它的格式：{1}"),
    ("WXGF image cannot be converted because the data is damaged: {}", "WXGF 图片无法转换，数据已损坏：{}"),
    ("WXGF image cannot be converted because its format is not supported yet: {}", "WXGF 图片无法转换，暂不支持它的格式：{}"),
    ("scanning messages", "正在扫描消息"),
    ("exporting {} media", "正在导出{}媒体"),
    ("reading messages", "正在读取消息"),
    ("writing formats", "正在写出各个格式"),
    // result fields and hints
    ("waiting for WeChat to start", "正在等待微信启动"),
    ("hook failed", "挂钩失败"),
    ("missing = media messages whose file is not on disk, as WeChat has not downloaded it, or could not be resolved; stickers need network access.\nthumbOnly = exported images that are only the thumbnail; open the original in WeChat, then export again for the HD image.", "missing = 文件不在磁盘上的媒体消息，即微信还没有下载，或无法解析；表情需要联网。\nthumbOnly = 导出的图片只有缩略图；请先在微信中打开原图，再重新导出以获得高清图。"),
    ("missing image identifier", "缺少图片标识"),
    ("cached image not found", "未找到缓存的图片"),
    ("image file not found, open the image in WeChat and retry", "未找到图片文件，请先在微信中打开该图片后重试"),
    ("image file not found", "未找到图片文件"),
    ("image decrypt key is not configured", "未配置图片解密密钥"),
    ("native decrypt failed, check the image keys: {}", "原生解密失败，请检查图片密钥：{}"),
    ("image cache", "图片缓存"),
    ("record id is empty", "记录 ID 为空"),
    ("Telegram push failed for chat {}: {}", "Telegram 推送到聊天 {0} 失败：{1}"),
    // backup and Weibo
    ("refusing to restore through the link {}: it may point outside {}", "拒绝通过链接 {0} 恢复：它可能指向 {1} 以外的位置"),
    ("cannot read {} of the account's database files, so no backup was written: {}", "无法读取账号的 {0} 个数据库文件，没有写出备份：{1}"),
    ("please enter a valid Weibo UID: digits only", "请输入有效的微博 UID：只能是数字"),
    // the command API's own answers (#1090)
    ("this server only answers requests to localhost, 127.0.0.1 or [::1]; to reach it by another name, start it with --host", "此服务只回答发往 localhost、127.0.0.1 或 [::1] 的请求；要用其他名字访问它，请用 --host 启动"),
    ("invalid or missing access token", "访问令牌无效或缺失"),
    ("no such command", "没有这个命令"),
    ("`{}` does not run in process; use /{}", "`{0}` 不在服务进程内运行；请使用 /{1}"),
    ("unknown parameter `{}`: `{}` takes no parameters", "未知的参数 `{0}`：`{1}` 不接受参数"),
    ("unknown parameter `{}` for `{}`; expected one of: {}", "`{1}` 没有参数 `{0}`；可用的参数：{2}"),
    ("unknown parameter `{}`", "未知的参数 `{}`"),
    ("parameter `{}` is a switch: pass true or false", "参数 `{}` 是开关：请传 true 或 false"),
    ("parameter `{}` takes one value, not several", "参数 `{}` 只接受一个值"),
    ("a parameter holds a NUL byte, which a command line cannot carry", "有参数含有 NUL 字节，命令行无法传递"),
    ("the request body must be a JSON object", "请求体必须是 JSON 对象"),
    ("request body too large", "请求体过大"),
    ("unknown std value `{}`; expected in, out or err", "未知的 std 值 `{}`；应为 in、out 或 err"),
    ("unknown return value `{}`; expected json or file", "未知的 return 值 `{}`；应为 json 或 file"),
    ("`{}` does not write a single file, so return=file is not available for it", "`{}` 写出的不是单个文件，不能使用 return=file"),
    ("several formats are written to a folder, which return=file cannot send: add zip=true to pack them into one zip", "多个格式会写进一个文件夹，return=file 无法发送：请加上 zip=true，打包成一个 zip"),
    ("return=file cannot be combined with std", "return=file 不能与 std 同时使用"),
    ("fields picks the columns of a --json document answered right away: not with std, return=file, `Prefer: respond-async` or a media route", "fields 只用于立即回答的 --json 文档：不能与 std、return=file、`Prefer: respond-async` 或媒体路由同时使用"),
    ("/api/ answers the --json document right away: std, return=file and `Prefer: respond-async` are for the routes without /api/", "/api/ 立即回答 --json 文档：std、return=file 和 `Prefer: respond-async` 只用于不带 /api/ 的路由"),
    ("an /api/ media route answers with the file itself: leave out `out`", "/api/ 的媒体路由直接回答文件本身：请去掉 `out`"),
    ("this /api/ media route does not take the request as it is: give each parameter once, as the command takes it", "这个 /api/ 媒体路由不接受这样的请求：请按命令的参数，每个参数只给一次"),
    ("std=out and std=err are answered right away; send them without `Prefer: respond-async`", "std=out 和 std=err 会立即回答；请不要带 `Prefer: respond-async`"),
    ("another task is running; retry later, or send `Prefer: respond-async` to queue it", "另一个任务正在运行；请稍后重试，或带上 `Prefer: respond-async` 排队"),
    ("too many commands are running; retry later, or send `Prefer: respond-async` to queue it", "正在运行的命令太多；请稍后重试，或带上 `Prefer: respond-async` 排队"),
    ("too many images, voices and videos are asked for; retry later", "请求的图片、语音和视频太多；请稍后重试"),
    ("too many reads are asked for; retry later", "请求的读取太多；请稍后重试"),
    ("failed to run weflow: {}", "运行 weflow 失败：{}"),
    ("cannot build the answer: {}", "无法生成回答：{}"),
    ("the address cannot be redirected", "无法重定向到这个地址"),
    ("method not allowed; use {}", "不支持这个请求方法；请使用 {}"),
    ("no such job", "没有这个任务"),
    ("the job has no file: it was not started with return=file, or has not succeeded", "这个任务没有文件：它不是用 return=file 启动的，或者还没有成功"),
    ("no WebUI is served; start serve with --ui-dir <a built WebUI>", "没有托管网页；请用 --ui-dir <构建好的网页> 启动 serve"),
    ("the WebUI's files are read-only; use GET", "网页的文件是只读的；请使用 GET"),
    ("no such file in the WebUI", "网页中没有这个文件"),
    // files: last, so that the messages above that begin the same way match first
    ("cannot create {}: {}", "无法创建 {}：{}"),
    ("cannot open {}: {}", "无法打开 {}：{}"),
    ("cannot read {}: {}", "无法读取 {}：{}"),
    ("cannot write {}: {}", "无法写入 {}：{}"),
    ("cannot remove {}: {}", "无法删除 {}：{}"),
    ("cannot lock {}: {}", "无法锁定 {}：{}"),
];

/// The Chinese of the English error `message`, with the texts its `{}` matched put in; `None` when it has no entry.
pub fn translate(message: &str) -> Option<String> {
    translate_with(CATALOG, message)
}

#[cfg(test)]
mod source_scan;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The text of every `.rs` file under `dir`, but this catalog's.
    fn sources_under(dir: &Path, text: &mut String) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources_under(&path, text);
            } else if path.extension().is_some_and(|e| e == "rs")
                && !path.ends_with("messages_zh.rs")
            {
                text.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }

    /// `source` with each string literal continued on the next line (a `\` at the end of the line) put on one line, as
    /// the literal's text is: the line break and the indent after it taken out.
    fn continued_lines_joined(source: &str) -> String {
        let source = source.replace("\r\n", "\n");
        let mut joined = String::with_capacity(source.len());
        let mut rest = source.as_str();
        while let Some(at) = rest.find("\\\n") {
            joined.push_str(&rest[..at]);
            rest = rest[at + 2..].trim_start_matches([' ', '\t']);
        }
        joined.push_str(rest);
        joined
    }

    #[test]
    fn a_continued_literal_is_joined_as_its_text_is() {
        assert_eq!(
            continued_lines_joined("(\"one, \\\r\n             two\", \"a\\\\b\")"),
            "(\"one, two\", \"a\\\\b\")"
        );
    }

    /// Every message the catalog translates is still said somewhere in the source: an entry whose message was
    /// reworded or removed would never be used again.
    #[test]
    fn every_entry_has_a_message_in_the_source() {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut sources = String::new();
        for krate in ["weflow-cli", "weflow-core", "weflow-native"] {
            sources_under(&crates.join(krate).join("src"), &mut sources);
        }
        let sources = continued_lines_joined(&sources);
        // a piece between placeholders as a string literal writes it
        let said = |piece: &str| {
            sources.contains(
                &piece
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n"),
            )
        };
        let used = |en: &str| {
            let longest = en
                .split("{}")
                .max_by_key(|p| p.len())
                .unwrap_or(en)
                .trim_matches(['`', '\'', ' ']);
            // a message of contexts joined by `: ` is put together from each of them
            en.split("{}").flat_map(|p| p.split(": ")).any(|piece| {
                // the code may put in the first words itself (`{what} cannot be converted…`), or the quotes around a
                // value with the value (`the WebUI package is refused: {what}`)
                let mut rest = piece.trim_matches(['`', '\'', ' ']);
                for _ in 0..3 {
                    if (rest.trim().chars().count() >= 12 || rest == longest) && said(rest) {
                        return true;
                    }
                    match rest.trim_start().split_once(' ') {
                        Some((_, tail)) => rest = tail,
                        None => break,
                    }
                }
                false
            })
        };
        let unused: Vec<&str> = CATALOG
            .iter()
            .map(|(en, _)| *en)
            .filter(|en| !used(en))
            .collect();
        assert!(
            unused.is_empty(),
            "no message in the source for {unused:#?}"
        );
    }

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
            translate("--start b is after --end a").as_deref(),
            Some("--start b 晚于 --end a")
        );
        assert_eq!(
            translate("invalid year 1969; use 0 for all years, or 1970-2100").as_deref(),
            Some("年份 1969 无效；0 表示所有年份，其余请使用 1970-2100")
        );
        assert_eq!(
            translate("invalid limit 101; use 1-100").as_deref(),
            Some("limit 101 无效；请使用 1 到 100")
        );
        assert_eq!(
            translate("unsupported sns export format: xlsx; supported: json, html, arkmejson")
                .as_deref(),
            Some("不支持的朋友圈导出格式：xlsx；支持：json、html、arkmejson")
        );
        assert_eq!(translate("not in the catalog"), None);
    }

    /// The command API's own answers (#1090), the templated ones with what the server puts in.
    #[test]
    fn the_command_api_s_own_errors_are_translated() {
        for (en, zh) in [
            (
                "unknown parameter `x` for `chat sessions`; expected one of: limit, offset",
                "`chat sessions` 没有参数 `x`；可用的参数：limit, offset",
            ),
            (
                "unknown parameter `x`: `health` takes no parameters",
                "未知的参数 `x`：`health` 不接受参数",
            ),
            ("unknown parameter `x`", "未知的参数 `x`"),
            (
                "method not allowed; use GET, POST",
                "不支持这个请求方法；请使用 GET, POST",
            ),
            (
                "`chat sessions` does not run in process; use /chat/sessions",
                "`chat sessions` 不在服务进程内运行；请使用 /chat/sessions",
            ),
            ("no such job", "没有这个任务"),
        ] {
            assert_eq!(translate(en).as_deref(), Some(zh), "{en}");
        }
    }

    /// The entries for any file come last: a message that begins the same way keeps its own entry.
    #[test]
    fn a_message_about_a_file_keeps_its_own_entry() {
        assert_eq!(
            translate(
                "cannot read 3 of the account's database files, so no backup was written: denied"
            )
            .as_deref(),
            Some("无法读取账号的 3 个数据库文件，没有写出备份：denied")
        );
        assert_eq!(
            translate("cannot lock WebUI 0.6.0: busy").as_deref(),
            Some("无法锁定网页 0.6.0：busy")
        );
        assert_eq!(
            translate("cannot read web/meta.json: denied").as_deref(),
            Some("无法读取 web/meta.json：denied")
        );
    }

    /// `config set <profile>.<key>` for a profile there is not names what it can suggest: nothing, `current_profile`,
    /// the setting spelt with an underscore, or both. Each way is told in Chinese.
    #[test]
    fn a_missing_profile_is_told_in_chinese_whatever_is_suggested() {
        let mut store = crate::config::ConfigStore::default();
        for (key, zh) in [
            ("serve.nothing", "没有名为 'serve' 的档案"),
            (
                "work.wxid",
                "没有名为 'work' 的档案；`config set current_profile work` 会建立这个档案",
            ),
            (
                "serve.port",
                "没有名为 'serve' 的档案；该设置应写作 serve_port",
            ),
            (
                "ai.model_api_key",
                "没有名为 'ai' 的档案；`config set current_profile ai` 会建立这个档案，该设置应写作 ai_model_api_key",
            ),
        ] {
            let err = store.set_key_from_text(None, key, "x").unwrap_err();
            assert_eq!(
                translate(&err.message).as_deref(),
                Some(zh),
                "{key}: {}",
                err.message
            );
        }
        assert_eq!(
            translate("there is no profile named 'zzz'; `config profiles` lists them").as_deref(),
            Some("没有名为 'zzz' 的档案；`config profiles` 会列出所有档案")
        );
    }

    #[test]
    fn webui_errors_keep_their_versions_and_reasons() {
        let backend = crate::webui::Backend {
            api_version: "2",
            version: "6.0.0",
        };
        let other_api = backend.refusal("1", "0.0.1").unwrap_or_default();
        assert_eq!(
            translate(&format!(
                "WebUI 0.4.1 cannot run with this weflow: {other_api}"
            ))
            .as_deref(),
            Some("网页 0.4.1 无法在这个 weflow 上运行：它需要 API 1，这个 weflow 提供 API 2")
        );
        let newer = backend.refusal("2", "9.0.0").unwrap_or_default();
        assert_eq!(
            translate(&newer).as_deref(),
            Some("它需要 weflow 9.0.0 或更新的版本，当前是 6.0.0")
        );
        assert_eq!(
            translate("cannot reach the WebUI registry https://ghcr.io for tags/list: timed out")
                .as_deref(),
            Some("无法访问网页仓库 https://ghcr.io，读取 tags/list 时出错：timed out")
        );
        assert_eq!(
            translate("not in the WebUI registry: manifests/0.9.9").as_deref(),
            Some("网页仓库中没有：manifests/0.9.9")
        );
        assert_eq!(
            translate("the WebUI package of 0.5.0 is damaged: expected 10 bytes with SHA-256 ab, got 9 bytes with cd").as_deref(),
            Some("网页 0.5.0 的安装包已损坏：应为 10 字节、SHA-256 为 ab，实际为 9 字节、SHA-256 为 cd")
        );
    }

    #[test]
    fn linux_ptrace_refusals_are_translated() {
        assert_eq!(
            translate("permission denied: cannot attach to WeChat, pid 42\nRun weflow as root, or allow ptrace through /proc/sys/kernel/yama/ptrace_scope.\nReason: Operation not permitted").as_deref(),
            Some("无法附加到微信进程，pid 42\n请以 root 运行 weflow，或通过 /proc/sys/kernel/yama/ptrace_scope 允许 ptrace。\n原因：Operation not permitted")
        );
        assert_eq!(
            translate("permission denied: weflow cannot attach to WeChat while /proc/sys/kernel/yama/ptrace_scope is 1\nRun weflow as root, or set ptrace_scope to 0.").as_deref(),
            Some("/proc/sys/kernel/yama/ptrace_scope 为 1 时，weflow 无法附加到微信进程\n请以 root 运行 weflow，或把 ptrace_scope 设为 0。")
        );
        assert_eq!(
            translate("permission denied: cannot read WeChat's memory, pid 42\nRun weflow as root, or allow ptrace through /proc/sys/kernel/yama/ptrace_scope.\nReason: denied").as_deref(),
            Some("无法读取微信的内存，pid 42\n请以 root 运行 weflow，或通过 /proc/sys/kernel/yama/ptrace_scope 允许 ptrace。\n原因：denied")
        );
    }

    #[test]
    fn an_unconverted_wxgf_image_keeps_its_date_and_reason() {
        assert_eq!(
            translate("WXGF image sent 2023-11-15 06:13:20 cannot be converted because its format is not supported yet: 10-bit samples").as_deref(),
            Some("2023-11-15 06:13:20 发送的 WXGF 图片无法转换，暂不支持它的格式：10-bit samples")
        );
        assert_eq!(
            translate("WXGF image cannot be converted because the data is damaged: no picture")
                .as_deref(),
            Some("WXGF 图片无法转换，数据已损坏：no picture")
        );
    }
}
