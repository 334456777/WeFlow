# `wcdb_api.dll`:新旧版本说明,以及 CLI 和新桌面端为什么不需要它

[English](../wcdb-api.md) | **简体中文**

## 要点

- **原生 Rust CLI**(`weflow`)和本仓库的**桌面端**(已改用 Rust 层,见 [desktop-rust-layer.md](desktop-rust-layer.md))
  **不加载、不内嵌、不打包** `wcdb_api.dll`(也不需要 `libwcdb_api.so`、`libwcdb_api.dylib`、`WCDB.dll`、`SDL2.dll`、
  `libWCDB.dylib`)。它们用仓库自己的纯 Rust 只读数据库层(`crates/weflow-native`)读取微信数据库;桌面端通过
  `weflow_wcdb`(`crates/weflow-wcdb-ffi`)使用这一层。构建、运行、打包它们都不需要 `resources/wcdb/` 里的任何文件。
- `resources/wcdb/` 留在仓库里,是给**原版桌面端**用的(即上游 WeFlow,它的 `electron/services/wcdbCore.ts` 加载
  `wcdb_api`;本仓库的 `wcdbCore.ts` 则通过同一套 C 接口加载 `weflow_wcdb`):用于补充、修复和维护原版,也作为参考。原作者提供的新版 `wcdb_api.dll` 已替换其中过期的
  那份;过期的旧文件保留在原位。

## 各文件说明

| 文件 | 版本 | 状态 |
|---|---|---|
| `resources/wcdb/win32/x64/wcdb_api.dll` | 原作者提供的新版(PE 时间戳 2026-07-07,sha256 `0904956c…a79350`) |  2099 年到期 |
| `resources/wcdb/win32/x64/wcdb_api.dll.old` | 旧版(PE 时间戳 2026-05-07,sha256 `6915913a…9d44b8`) | 2026-09-30 23:59:59(本地时间)之后失效:`wcdb_init` 返回 `-1000`,并登记删除自身。原样保留,供研究。 |
| `resources/wcdb/win32/arm64/wcdb_api.dll`、`resources/wcdb/linux/x64/libwcdb_api.so`、`resources/wcdb/macos/universal/libwcdb_api.dylib` | 旧版 | 未替换:这些平台没有新版。是否带有同样的到期时间,没有检查过。 |
| `WCDB.dll`、`SDL2.dll`、`libWCDB.dylib` | 未变 | `wcdb_api` 的依赖;新版使用同一个 `WCDB.dll`。 |

各部分是否用到这些库:

| | 加载 `wcdb_api` | 打包 `resources/wcdb/` |
|---|---|---|
| 原生 Rust CLI(`weflow`) | 否 | 否(`crates/weflow-assets/build.rs` 从不内嵌 `resources/wcdb/`) |
| 本仓库的桌面端(Rust 层) | 否(`wcdbCore.ts` 加载 `weflow_wcdb`) | 否(`package.json` 排除了 `wcdb/**`) |
| 原版桌面端(上游 WeFlow,以及本仓库改用 Rust 层之前的版本) | 是 | 是 |

## x64 新旧两版的差别

以下来自文件本身(导出表和导入表),没有拿它们读取数据:

- **新增导出**:`wcdb_add_custom_emoticon`、`wcdb_update_custom_emoticon`、`wcdb_delete_custom_emoticon`、
  `wcdb_get_db_status`、`wcdb_probe_fts_schema`、`wcdb_purge_memory`。其中仓库的 `wcdbCore.ts` 只用到
  `wcdb_get_db_status`(可选绑定),`weflow_wcdb` 也实现了它。
- **去掉的导出**:`wcdb_open_message_cursor_lite`、`wcdb_open_message_cursor_lite_with_key`。原版 `wcdbCore.ts`
  把前者作为可选绑定,缺少时会改用 `wcdb_open_message_cursor`,所以原版桌面端仍能工作,只是游标返回完整行而不是精简行。
  `weflow_wcdb` 仍然导出它。
- **依赖相同**:`WCDB.dll`、Visual C++ 运行库和 WinHTTP。两版都链接了 WinHTTP,并包含原作者一个上报服务的地址;
  是否发送、发送什么,没有核实。Rust 层没有任何联网代码。

## 与 Rust 层的对照验证

2026-10-01 尝试过:通过原版 `wcdbCore.ts` 调用同一组读取函数(会话、消息、游标、联系人、群、统计、报告、朋友圈、搜索、
媒体、hardlink、表浏览,约 60 个调用),一次用新版 `wcdb_api.dll`,一次用 `weflow_wcdb`,各自使用同一个真实账号数据库的
独立副本,然后比较结果。

结果:新版 `wcdb_api.dll` 能加载,`InitProtection` 也成功,但 `wcdb_open_account` 返回 **`-1005`**:库自带的安全校验
不接受这个测试程序,因此没有通过它读到任何数据,对照无法进行。我们没有尝试绕过这个校验。要做这项对照,需要在它所面向的
宿主(原版桌面端)里运行,或者需要原作者协助;这一点列在下面的讨论里。

Rust 层另有自己的验证(测试套件里的合成加密账号、一个真实的 Windows 账号、不同版本之间输出逐字节比对),见
[cli-unsupported.md](cli-unsupported.md#尚待验证)。

## 讨论

关于 `wcdb_api.dll` 的问题在这个 issue 里讨论:

- Issue: [hicccc77/WeFlow#1220](https://github.com/hicccc77/WeFlow/issues/1220#issuecomment-5925015014)
