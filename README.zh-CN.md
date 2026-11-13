# WeFlow CLI
[English](README.md) | **简体中文**

`weflow` 是 [WeFlow](https://github.com/hicccc77/WeFlow) 后端的 Rust 原生命令行版本，直接在终端读取、分析和导出本地的微信 4.0 及以上版本聊天记录。

- 默认输出便于阅读的文本（对齐的 `键: 值` 和表格；错误写到 stderr）。加 `--json` 则在 stdout 输出一个 JSON 文档（`{"success": true, "data": ...}`），方便脚本处理。
- 会话、消息、联系人、朋友圈，私聊/群聊统计分析，年度报告与双人报告。
- 消息导出支持 9 种格式：`txt`、`json`、`arkme-json`、`chatlab`、`chatlab-jsonl`、`excel`、`weclone`、`html`、`sql`。
- 图片（`.dat` 解密）、语音（SILK → WAV）、视频查找、表情。
- 所有命令的 HTTP 接口（`serve`）、消息推送、AI 见解。
- `serve` 在 `/ui/` 下提供网页界面（WebUI，[详情](https://github.com/334456777/WeFlow/wiki/WebUI#网页)）。
- `--help`、参数错误、运行错误和生成的文本默认跟随系统语言(中文或英文);`weflow lang en|zh` 可保存选择。

## 安装

从 [Releases](https://github.com/334456777/WeFlow/releases/latest) 下载对应平台的压缩包，解压后把里面的 `weflow` 放进 `PATH` 中的目录。

也可以从源码构建并安装到 `PATH` 中的目录（macOS 和 Linux 为 `/usr/local/bin`，Windows 为 `%USERPROFILE%\.cargo\bin`，在 Git Bash 中运行）：

```bash
make install
weflow --help
```

## 切换中英文

```bash
weflow lang zh
```

语言会影响 `--help`、参数错误、运行时错误和生成的文本;JSON 键和错误码仍为英文。完整的优先级见[命令参考](https://github.com/334456777/WeFlow/wiki/命令参考#语言)。

## 首次设置与导出（必要步骤）

请**以管理员身份运行 PowerShell**，微信上至少登录过一次微信账号，发送过消息，打开过图片，支持微信 4.0 及以上版本。

1. 设置微信数据目录
```powershell
weflow db detect
weflow config set db_path "C:\Users\<你>\Documents\xwechat_files"
```
> stdout: `db_path: C:\Users\<你>\Documents\xwechat_files`

2. 设置你的 wxid
```powershell
weflow db wxid
weflow config set wxid wxid_xxxxxxxx
```
> stdout: `wxid: wxid_xxxxxxxx`

3. 获取并保存数据库密钥（详细见下文）
```powershell
weflow key db --save
```
> stdout: `已获取数据库密钥，已保存到当前档案`

4. 获取并保存图片密钥（目前只支持 Windows）
```powershell
weflow key image --save
```
> stdout: `已获取图片密钥，已保存到当前档案`

不加 `--save` 时，命令输出密钥而不保存，可以用 `weflow config set` 手动保存。

5. 确认数据库可以打开
```powershell
weflow db test
```

6. 尝试列出会话并导出聊天记录（私聊的会话 ID 是对方的 wxid，群聊是 `xxx@chatroom`）
```powershell
weflow chat sessions
weflow export messages <会话 ID> --format html --out chat.html
```

需要导出为其他格式（`txt`、`json`、`excel`、`sql` 等）、按日期或发送者筛选、连同图片等媒体一起导出，见 Wiki 的[消息导出](https://github.com/334456777/WeFlow/wiki/消息导出#格式)。

也可以运行 `weflow serve`，在浏览器中打开网页界面。

`weflow config list` 查看已保存的配置，`weflow config path` 查看配置文件的位置。

---
**`weflow key db` 做了什么？** <br>数据库密钥在微信打开数据库时在内存中生成，从对应内存位置提取 `decrypt_key`:

```
检测到微信正在运行（pid 31912）请先完全退出微信：
右下角托盘图标 → 右键 → 退出微信
等待微信退出…（剩余 178 秒自动退出）
微信已退出，请重新打开微信。
等待微信启动…（剩余 171 秒自动退出）
检测到微信（pid 20816）请在登录窗口点击「进入微信」。
等待密钥…（剩余 150 秒自动退出）
已获取数据库密钥，已保存到当前档案
```
`180 秒后自动退出` 可用 `weflow key db --timeout <秒>` 调整

## CLI 不支持的功能

WeFlow Rust CLI ~~因为本人技艺不精湛所以~~**只读**数据库，不会修改微信数据库~~有些锅我不背~~，会修改微信数据库的命令(`chat update-message`、`chat delete-message`、
`chat anti-revoke`、`chat mark-read`、`sns block-delete`、`sns delete`)设计为占位符，另有一些 WeFlow 桌面端的功能缺失
(语音转文字、弹窗等桌面进程功能)。

详细清单见[不支持的功能](https://github.com/334456777/WeFlow/wiki/不支持的功能)。

## 文档

[WeFlow Wiki](https://github.com/334456777/WeFlow/wiki)

# 免责声明
<div id="disclaimer">

## 1. 项目目的与性质
项目（以下简称“本项目”）是作为一个技术研究与学习工具而创建的，旨在探索和学习文本统计与分析技术。本项目基于[Attention Is All You Need](https://arxiv.org/abs/1706.03762)，纯[SI](https://www.whitehouse.gov/presidential-actions/2026/09/inaugurating-the-era-of-super-intelligence/)全自动生成0%人工成分。

<table>
  <tr>
    <td><img src="docs/images/SSD.JPG" alt="SSD" width="150"></td>
    <td><img src="docs/images/PMM.JPG" alt="PMM" width="150"></td>
    <td><img src="docs/images/HDD.JPG" alt="HDD" width="150"></td>
    <td><img src="docs/images/MONITOR.JPG" alt="MONITOR" width="150"></td>
  </tr>
</table>

<table>
  <tr>
    <td><img src="docs/images/MB.JPG" alt="MB" width="150"></td>
    <td><img src="docs/images/GPU.JPG" alt="GPU" width="150"></td>
    <td><img src="docs/images/RAM.JPG" alt="RAM" width="150"></td>
  </tr>
</table>

## 2. 法律合规性声明
本项目开发者（以下简称“开发者”）郑重提醒用户在下载、安装和使用本项目时，严格遵守中华人民共和国相关法律法规，包括但不限于《中华人民共和国网络安全法》、《中华人民共和国反间谍法》等所有适用的国家法律和政策。用户应自行承担一切因使用本项目而可能引起的法律责任。

## 3. 使用目的限制
本项目严禁用于任何非法目的或非学习、非研究的商业行为。本项目不得用于任何形式的非法侵入他人计算机系统，不得用于任何侵犯他人知识产权或其他合法权益的行为。用户应保证其使用本项目的目的纯属个人学习和技术研究，不得用于任何形式的非法活动。

## 4. 收集的数据
本项目不会收集、存储或传输任何用户数据，所有操作均在本地进行。用户在使用本项目时，应确保其行为符合相关法律法规的要求。

## 5. 免责声明
开发者已尽最大努力确保本项目的正当性及安全性，但不对用户使用本项目可能引起的任何形式的直接或间接损失承担责任。包括但不限于由于使用本项目而导致的任何数据丢失、设备损坏、法律诉讼等。

## 6. 知识产权声明
WeFlow项目的知识产权归[hicccc77](https://github.com/hicccc77)开发者所有。本项目受到著作权法和国际著作权条约以及其他知识产权法律和条约的保护。用户在遵守本声明及相关法律法规的前提下，可以下载和使用本项目。

## 7. 最终解释权
关于本项目的最终解释权归[hicccc77](https://github.com/hicccc77)开发者所有。开发者保留随时更改或更新本免责声明的权利，恕不另行通知。
</div>
