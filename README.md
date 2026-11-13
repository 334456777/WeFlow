# WeFlow CLI

**English** | [简体中文](README.zh-CN.md)

`weflow` is a Rust command-line version of the [WeFlow](https://github.com/hicccc77/WeFlow) backend. It reads, analyzes, and exports local WeChat chat history (WeChat 4.0 and later) right in the terminal.

- Prints human-readable text by default; add `--json` to get JSON for scripts.
- Supports [sessions and messages](https://github.com/334456777/WeFlow/wiki/聊天与媒体命令#会话与消息), [contacts](https://github.com/334456777/WeFlow/wiki/聊天与媒体命令#联系人与群), [Moments](https://github.com/334456777/WeFlow/wiki/朋友圈), [private and group chat analytics](https://github.com/334456777/WeFlow/wiki/统计与报告), [annual reports](https://github.com/334456777/WeFlow/wiki/统计与报告#年度报告), and [dual reports](https://github.com/334456777/WeFlow/wiki/统计与报告#双人报告).
- Exports messages in 9 formats: `txt`, `json`, `arkme-json`, `chatlab`, `chatlab-jsonl`, `excel`, `weclone`, `html`, and `sql`. Images, voice messages, videos, and stickers can be exported too.
- `weflow serve` provides an HTTP API for every command and a web interface ([WebUI](https://github.com/334456777/WeFlow/wiki/WebUI)).

## Install

Download the archive for your platform from [Releases](https://github.com/334456777/WeFlow/releases/latest). Once unpacked, the `weflow` inside is ready to use.

Or build from source and install:

```bash
make install
weflow --help
```

Uninstall: `make uninstall`

## Switching between English and Chinese

```bash
weflow lang en
```

For language settings and precedence, see [Command reference - Language](https://github.com/334456777/WeFlow/wiki/命令参考#语言) on the Wiki.

## First-time setup and export (required steps)

**Run PowerShell as administrator.** WeChat 4.0 and later are supported. You need to have logged in to an account at least once, sent messages, and opened images.

1. Set the WeChat data directory
```powershell
weflow db detect
weflow config set db_path "C:\Users\<you>\Documents\xwechat_files"
```
> stdout: `db_path: C:\Users\<you>\Documents\xwechat_files`

2. Set your wxid
```powershell
weflow db wxid
weflow config set wxid wxid_xxxxxxxx
```
> stdout: `wxid: wxid_xxxxxxxx`

3. Get and save the database key ([details below](#key-db))
```powershell
weflow key db --save
```
> stdout: `Database key obtained and saved to the active profile`

4. Get and save the image keys (Windows only for now)
```powershell
weflow key image --save
```
> stdout: `Image keys obtained and saved to the active profile`

Without `--save`, the commands only print the keys; you can save them yourself with `weflow config set`.

5. Check that the database opens
```powershell
weflow db test
```

6. Try listing the conversations and exporting the chat history (a private chat's conversation ID is the other person's wxid; a group's is `xxx@chatroom`)
```powershell
weflow chat sessions
weflow export messages <conversation-id> --format html --out chat.html
```

To export to other formats (`txt`, `json`, `excel`, `sql`, and more), filter by date or sender, or export images and other media along with the messages, see [Message export - Formats](https://github.com/334456777/WeFlow/wiki/消息导出#格式) on the Wiki.

`weflow config` manages the configuration file.

---
<a id="key-db"></a>**What does `weflow key db` do?** <br>The database key is generated in memory when WeChat opens its databases, and the command extracts `decrypt_key` from that memory location:

```
WeChat is running (pid 31912). Quit it completely first:
system tray icon -> right click -> Quit WeChat
Waiting for WeChat to quit... (exits automatically in 178 s)
WeChat has quit. Open it again.
Waiting for WeChat to start... (exits automatically in 171 s)
WeChat found (pid 20816). Click "Enter WeChat" in the login window.
Waiting for the key... (exits automatically in 150 s)
Database key obtained and saved to the active profile
```
The 180-second automatic exit can be adjusted with `weflow key db --timeout <seconds>`.

For the full flow and how it works on each platform, see [Key acquisition - Database key](https://github.com/334456777/WeFlow/wiki/密钥获取#数据库密钥) on the Wiki.

## What the CLI does not support

The WeFlow Rust CLI opens databases **read-only** and never modifies WeChat databases; commands that would modify a database are only placeholders. Some features of the WeFlow desktop app (desktop-process features such as voice-to-text and popups) are not implemented either.

For the detailed list, see [Unsupported features](https://github.com/334456777/WeFlow/wiki/不支持的功能) on the Wiki.

## Documentation

[WeFlow Wiki](https://github.com/334456777/WeFlow/wiki)

# Disclaimer
<div id="disclaimer">

## 1. Purpose and nature of the project
This project (hereinafter "the Project") was created as a tool for technical research and learning, to explore and study text statistics and analysis techniques. The Project is based on [Attention Is All You Need](https://arxiv.org/abs/1706.03762) and was generated fully automatically by pure [SI](https://www.whitehouse.gov/presidential-actions/2026/09/inaugurating-the-era-of-super-intelligence/), with 0% human input.

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

## 2. Legal compliance
The developer of the Project (hereinafter "the Developer") solemnly reminds users to strictly comply with the relevant laws and regulations of the People's Republic of China when downloading, installing, and using the Project, including but not limited to the Cybersecurity Law of the People's Republic of China, the Counter-Espionage Law of the People's Republic of China, and all other applicable national laws and policies. Users bear sole responsibility for any legal liability that may arise from their use of the Project.

## 3. Restrictions on use
The Project must not be used for any illegal purpose or for any commercial activity other than learning and research. It must not be used to illegally intrude into another person's computer system in any form, or to infringe anyone's intellectual property rights or other lawful rights and interests. Users must ensure that they use the Project solely for personal learning and technical research, and not for any form of illegal activity.

## 4. Data collected
The Project does not collect, store, or transmit any user data; everything runs locally. When using the Project, users must ensure that their conduct complies with the relevant laws and regulations.

## 5. Disclaimer
The Developer has made every effort to ensure the legitimacy and safety of the Project, but accepts no liability for any direct or indirect loss of any kind arising from its use, including but not limited to data loss, device damage, and legal proceedings.

## 6. Intellectual property
The intellectual property rights of the WeFlow project belong to its developer, [hicccc77](https://github.com/hicccc77). The Project is protected by copyright law, international copyright treaties, and other intellectual property laws and treaties. Users may download and use the Project provided that they comply with this statement and the relevant laws and regulations.

## 7. Final interpretation
The right of final interpretation of the Project belongs to its developer, [hicccc77](https://github.com/hicccc77). The Developer reserves the right to change or update this disclaimer at any time without notice.
</div>
