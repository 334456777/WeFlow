# 验证计划

[English](../verification-plan.md) | **简体中文**

这里记录还没有用真实数据验证过的部分,以及打算怎么验证。原生数据库层、各种导出和其他命令已经在一个真实的 Windows 微信 4.x 账号上验证过(Linux 构建和 Windows 的 `weflow.exe` 都跑过);下面是剩下的缺口。每完成一项,就在这里记下结果,并同步更新 [cli-unsupported.md](cli-unsupported.md) 第 4 节。

| # | 项目 | 状态 | 验证方法 | 完成标准 |
|---|---|---|---|---|
| 1 | **macOS 和 Linux 真实账号** | 计划中 | 在各平台上:`key db`(或对应平台的密钥辅助程序)、`db detect`/`db test`,然后跑回归脚本(会话、消息、联系人、朋友圈、报告、每种导出格式加 `--media`、`export media`、HTTP API),并与桌面端对比数量。 | 两个平台的回归都通过、没有意外失败;差异要么修复,要么写进 cli-unsupported.md。 |
| 2a | **Windows 图片自动下载钩子**(`image auto-download start`、`serve --image-auto-download`) | 计划中 | 在运行着微信的 Windows x64 上:启动钩子,打开含有从未下载过图片的聊天,确认文件出现在 `msg/attach/…/Img` 下,之后 `export media` 能找到它们;停止钩子,确认微信工作正常。 | 钩子运行期间图片会被下载,停止后不再有动作,微信不受影响。 |
| 2b | **AI 洞察对接真实服务商**(`insight test`、`insight trigger`、`serve --insight`、足迹总结) | 计划中 | 为一个 OpenAI 兼容的服务商配置 `ai_model_api_base_url`、`ai_model_api_key`、`ai_model_api_model`;运行 `insight test`、手动触发和足迹总结;检查请求(`/chat/completions`,没有多余的 `/v1`)、解析出的回答和保存的记录;可选验证 Telegram 推送。 | 所有洞察命令在一个真实服务商上端到端可用;服务商返回的错误能清楚地报出来。 |
| 3 | **备份与桌面端是否兼容** | 计划中 | 分别用桌面端和 `weflow backup create` 生成备份;用 `weflow backup inspect` 查看两者;用另一个工具把各自的备份恢复到空目录,比较文件列表和哈希;用 `db test` 和桌面端打开恢复出的账号。 | 两个方向恢复出的文件一致,或者差异有记录并说明原因。 |

另有一项工作在别的分支进行:桌面端仍在加载 `wcdb_api.dll`,正在 `claude/desktop-rust-layer` 分支里改为使用 Rust 层。
