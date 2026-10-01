# 桌面端改用 Rust 数据库层

[English](../desktop-rust-layer.md) | **简体中文**

桌面端(Electron)原来通过闭源的 `wcdb_api` 库读取微信数据库,这个库在 2026-09-30 之后失效(`wcdb_init` 返回 `-1000`)。在这个分支上,桌面端改为加载 **`weflow_wcdb`**:一个由 `crates/weflow-wcdb-ffi` 构建的 C 接口库,底层是与 CLI 相同的纯 Rust 只读数据库层。

## 结构

- `crates/weflow-wcdb-ffi` 导出 `electron/services/wcdbCore.ts` 声明的函数(`wcdb_open_account`、`wcdb_get_sessions`、`wcdb_open_message_cursor` 等,92 个里的 88 个),参数、状态码和 JSON 结果都与原来一致。字符串仍通过 `_Out_ void**` 参数返回,并用 `wcdb_free_string` 释放。
- `wcdbCore.ts` 只改了查找库文件的位置:`resources/native-db/<平台>/<架构>/weflow_wcdb.dll`(`libweflow_wcdb.so`、`libweflow_wcdb.dylib`),开发时也会找 `target/release/`,或者用 `WCDB_DLL_PATH` 指定。不再预加载 `WCDB.dll` / `SDL2.dll` / `libWCDB.dylib`。
- 打包时带上 `resources/native-db/`,不再带 `resources/wcdb/`。这些库(新版 `wcdb_api.dll` 和过期的旧版)留在仓库里给原版桌面端用,见 [wcdb-api.md](wcdb-api.md)。
- 变更通知:`wcdb_start_monitor_pipe` 打开命名管道(Windows)或 Unix 套接字,每当数据库有变化就发一行 JSON(`session_change`、`message_change`、`contact_change`),变化通过每秒检查一次文件得到。桌面端原有的管道客户端不用改就能收到。

## 构建

```
npm run native-db:build          # cargo build -p weflow-wcdb-ffi,并复制到 resources/native-db/
npm run build                    # 先执行 native-db:build,再执行 tsc、vite 和 electron-builder
```

交叉编译,例如在 Linux 上编译 Windows 库:`node scripts/build-native-db.cjs --target x86_64-pc-windows-gnu`(需要 MinGW 工具链)。

## 与旧库的差别

| 方面 | 行为 |
|---|---|
| 编辑/删除消息、防撤回和朋友圈防删触发器、"全部标为已读"、删除朋友圈、导入表 | 以状态码 `-4` 和"只读"说明拒绝;触发器检查报告"未安装"。界面会显示这个错误。 |
| `InitProtection`、`wcdb_init`、云端上报(`wcdb_cloud_*`) | 接受但什么都不做:没有有效期检查,也不联网。 |
| `VerifyUser`(旧库里的 Windows Hello 验证) | 不导出;桌面端会把这个函数视为不可用。 |
| 变更监听 | 每秒检查文件大小和修改时间,而不是 `ReadDirectoryChangesW`;事件只带数据库路径,不带具体的表或会话。 |
| 群成员 | 以数组返回,带有联系人表里的 `avatarUrl`。 |

## 验证情况

- C 接口的 Rust 测试,用合成的加密账号(`cargo test -p weflow-wcdb-ffi`),包括监听套接字。
- 在 Windows 上用 Node 的 `koffi` 加载 `weflow_wcdb.dll`,对真实账号测试:打开、会话、消息、20 万条消息的游标、联系人、头像、年度报告、写操作被拒绝,以及连接监听管道。
- 把桌面端自己的 `wcdbCore.ts` 打包后在 Node 下运行,用 Linux 库对真实账号测试:约 60 个调用(会话、消息、计数、游标、联系人、群、统计、报告、朋友圈、搜索、媒体流、hardlink、表浏览、监听)全部成功,写操作按预期被拒绝。
- `tsc -p tsconfig.node.json` 没有新增错误。
- **还没做:** 在 Windows、macOS、Linux 上运行打包后的桌面端界面(见 [plan.md](plan.md#还需要验证的部分) 第 4 项)。
