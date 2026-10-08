# Windows 数据库密钥捕获

[English](../windows-db-key.md)

Windows x64 `key db` 使用 `weflow-native/src/windows_db_key/` 中的 Rust 实现。密钥参数结构（RDX 指向结构，+8 为指针，+16 为字节数）已与 [ycccccccy/wx_key](https://github.com/ycccccccy/wx_key) 源码及维护者提供的原 DLL 反编译材料核对；源码遵循其 [MIT 许可证](../../crates/weflow-native/src/windows_db_key/LICENSE)。

定位按原二进制的三段版本范围选择一个特征：4.1.4 之前、4.1.4–4.1.6.14、4.1.6.14 之后。最新范围复用较旧的字节特征，但捕获偏移不同。仅扫描已加载的 `.text` 段，要求唯一匹配，并用 [AMD64 异常目录](https://learn.microsoft.com/en-us/cpp/build/exception-handling-x64) 检查捕获点与特征所在范围。远程读取前检查头部和区间边界，扫描使用带重叠的 1 MiB 分块。只保留一条定位路径，不设置兜底方法。

捕获通过公开的 [Windows 调试 API](https://learn.microsoft.com/en-us/windows/win32/api/debugapi/nf-debugapi-debugactiveprocess) 连接进程，在已有和新建线程的空闲硬件断点槽位设置执行断点。保留其他槽位，转交无关异常，只读取 32 字节密钥，并在输出密钥前恢复寄存器、[解除连接](https://learn.microsoft.com/en-us/windows/win32/api/debugapi/nf-debugapi-debugactiveprocessstop)。不分配可执行内存，不修改客户端指令。超时、显式清理、Ctrl+C 和进程退出均由同一工作线程处理。

仅支持 x64 微信 4.x。原二进制以 `4.1.6.14` 为界的特征表不同于早期备份源码，已补充边界回归测试。其通用 Cipher 定位在 4.1.13.65 上不满足 256 字节对齐条件，因此此处没有采用该策略。未来版本仍需实测确认特征与参数结构。Windows ARM64 构建不会加载 x64 DLL 代替。

CLI 直接链接捕获代码。`weflow-wxkey` 为桌面端导出原 C++ bool 接口（`InitializeHook`、`PollKeyData`、`CleanupHook`、`GetStatusMessage`、`GetLastErrorMsg`、`GetImageKey`）。`GetImageKey` 通过 #95 的 Rust 解析器返回候选 code，调用方仍须用选定账号样本验真。`npm run native-db:build` 构建数据库库，并在 Windows x64 上把 Rust 密钥 DLL 构建到 `resources/native-key/win32/x64/`。卸载 DLL 前须先清理。不保留厂商库兜底，也没有切换实现的环境变量。

已有调试器、硬件断点槽位耗尽或进程权限不足会明确报错。仓库没有提供证书，构建产物未签名。安全软件可能限制调试器连接：请查看其实际拦截记录，仅对可信构建放行；实现不会更改杀软设置，也不使用间接系统调用绕过监测。

自动测试使用独立的合成 x64 进程，覆盖主线程与新线程、无效密钥参数、函数返回值、取消、重复连接、进程退出、调试器与寄存器恢复。真实客户端验收仍需记录版本与架构，在登录时捕获，用 `db test` 验真，并确认清理后微信仍可正常使用。真实账号标识、密钥、路径与聊天内容不得进入公开测试产物。

已在 Windows x64 微信 4.1.13.65 上验证：CLI 与 Rust DLL 分别在重新登录时捕获到相同密钥；`db test` 通过，改变密钥后被拒绝，调试连接已解除，检查到的 168 个线程均无遗留启用断点。验证范围为一个账号和一个客户端版本。
