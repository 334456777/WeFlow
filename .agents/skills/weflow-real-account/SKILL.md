---
name: weflow-real-account
description: 在本机的真实微信账号上验证、测量或调研 WeFlow：真实数据的禁忌（只读、不联网下载）、隔离的 WEFLOW_HOME、新旧二进制 --json 的逐项对比、用探针调研表结构和字段含义、只公开汇总、开发时的密钥文件、加固前先实测失败率，以及用完的清理。碰真实微信数据、在真实账号上运行 weflow、做改动前后的对比、调研微信的表结构或某个字段和位的含义、碰后端的 PR 合并前验证时使用。
---

# 在真实账号上验证

合成账号覆盖不到真实数据的一些情况（例如有多个 `message_N.db` 时 `local_id` 冲突，[#144](https://github.com/334456777/WeFlow/issues/144)），所以碰后端的功能 PR 在合并前要在真实账号上跑一遍，把结果的汇总写进 PR 正文或评论。Wiki 的说明见 [测试与基准#真实数据回归](https://github.com/334456777/WeFlow/wiki/测试与基准#真实数据回归)。

## 先过一遍这些禁忌

- 只读。不建目录联接或符号链接指向真实账号的目录：删除链接时可能删掉真实数据。需要副本就真的复制，放在工作区内被忽略的目录（如 `target/` 下），用完删除。
- 不联网下载：
  - 导出不用 `--media all` 或 `emoji`（会下载表情），只用 `image,voice,video`；`file` 只读本机文件，也可以用。
  - `export media` 的 `--type` 默认是 `all`，必须显式写 `--type image,voice,video`。
  - `--media avatar`（`all` 包含它）会下载本机没有的头像，需要加 `--no-avatar-download`，只用 `head_image.db` 里的头像。
  - `chat emoji` 会下载表情。
  - `sns export --format html` 不带 `--media` 也会下载每个发帖人的头像，每次下载到的个数不同：真实账号上只比较 `json` 和 `arkmejson`，`html` 在合成账号上比较。
  - 会写文件、联网或调用 AI 的命令只在合成账号上运行。
- 长时间运行的后台脚本里不放会联网的选项。停掉后台任务后确认它的整个进程树已经结束再重跑：在 Windows 的 Git Bash 里停掉任务不一定结束它启动的脚本和子进程，两个任务会互相干扰，数字也会被污染。
- 公开的只有汇总：状态码、数量、大小、耗时。不打印、不提交名字、wxid、路径、消息内容和密钥；命令行里有会话 id 时用标签代替，出错时先替换掉再打印。

## 隔离的配置

[`scripts/isolated_home.py`](scripts/isolated_home.py) 把配置复制到一个新目录，并把各档案的缓存路径置空；命令用 `WEFLOW_HOME` 指向这个目录，真实的配置和缓存都不会被改动：

```sh
python .agents/skills/weflow-real-account/scripts/isolated_home.py target/real-verify/home
WEFLOW_HOME=target/real-verify/home weflow --json chat sessions
```

缓存会挡住新下载的结果，做导出对比时每次用新的目录。

## 改动前后对比

1. `main` 和分支各编一个二进制，各用单独的 `CARGO_TARGET_DIR`，做法和坑见 [weflow-perf](../weflow-perf/SKILL.md#新旧二进制)。
2. 把要比较的命令写进一个文件，每行一条（如 `analytics overall`），用 [`scripts/compare_commands.py`](scripts/compare_commands.py) 比较两个二进制 `--json` 的结果。它去掉每次都会变的键（`updatedAt`、`elapsed_ms` 等），区分数值类型，只打印相同和不同的条数、退出码与耗时，输出可以直接公开。

   ```sh
   python .agents/skills/weflow-real-account/scripts/compare_commands.py <旧二进制> <新二进制> target/real-verify/home <命令文件>
   ```

3. 预期的差异：导出文件名里的时间戳、用法提示里的程序名。
4. 导出逐字节比较时，xlsx 先抹掉 sharedStrings 和流式 inlineStr 两处的导出时间。
5. 消息读取的游标另有探针 `real_account_probe`，用法见上面的 Wiki 一节。

## 调研表结构

查微信把某样东西存在哪张表、哪一列、哪一位时（如会话置顶，[#885](https://github.com/334456777/WeFlow/issues/885)），用各 crate `examples/` 下的探针，列表见 Wiki [测试与基准#探针](https://github.com/334456777/WeFlow/wiki/测试与基准#探针)。

- 先看结构：`cargo run -p weflow-native --example schema_probe -- <账号目录> <密钥文件> <db_storage 下的相对路径>` 只输出表、列和行数，不输出内容。
- `sql_probe` 原样逐行输出查询结果：只查聚合（`count`、`group by`、位运算的分布）和结构，不 `select` 名字、wxid、消息内容这类列。
- 找某个状态对应的字段：在微信里改一次这个状态（如置顶一个会话），前后各运行一次探针，比较两次输出的差别。
- 现有的探针不够用时写一个新的，放在 `weflow-core` 或 `weflow-native` 的 `examples/<名>_probe.rs`：
  - 只读，`//!` 写明用途、用法和它输出什么、不输出什么；
  - 只输出汇总和诊断，会话用它在列表中的位置、类型和用户名的短哈希表示（参照 `pinned_probe`）；
  - 登记到上面的 Wiki 表里。
- 结论按「结论、依据、原因」评论到对应的 issue（[weflow-issues](../weflow-issues/SKILL.md#写正文和评论)），只写汇总。能在合成账号上复现的，把表结构加进 `fixture` 再写测试，见 [weflow-tests](../weflow-tests/SKILL.md#端到端测试和合成账号)。

## 密钥

- 开发和调研时（探针、真实账号上的调研），数据库密钥和派生密钥可以临时写到工作区内被忽略的目录，用完立刻删除并说明；不打印、不提交、不上传。
- 正式功能里的密钥只在内存里传递（例如用 stdin 管道交给子进程），不写到磁盘。维护者允许开发时落盘，不等于选定为产品的做法；拿不准时把取舍摆给维护者选择。

## 加固之前先实测

开启证书校验、收紧输入检查这类改动，先写临时探针在真实的地址和数据上实测：校验与不校验各跑几轮，按主机和原因统计失败，只输出汇总。探针用程序同一套 TLS（`weflow_core::http_client::builder()`），用完删除，不提交。失败多到影响下载或导出时，不做这项加固，或者保留不校验的回退；现有的例外见 `crates/weflow-core/src/http_client.rs` 的 `STICKER_HOSTS_WITH_A_WRONG_NAME`。

## 收尾

删掉隔离的配置目录、导出的文件、对比用的二进制和含密钥的文件，确认没有残留的 weflow 进程。
