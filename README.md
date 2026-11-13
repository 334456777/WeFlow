# WeFlow CLI

**English** | [简体中文](README.zh-CN.md)

`weflow` is a native Rust command-line build of the [WeFlow](https://github.com/hicccc77/WeFlow) backend. It reads, analyzes, and exports local WeChat 4.0+ chat history directly from the terminal.

- Human-readable text by default (aligned `key: value` fields and tables; errors go to stderr). Add `--json` to print a single JSON document on stdout (`{"success": true, "data": ...}`) for scripting.
- Sessions, messages, contacts, and Moments; private and group chat analytics; annual reports and dual reports.
- Message export in 9 formats: `txt`, `json`, `arkme-json`, `chatlab`, `chatlab-jsonl`, `excel`, `weclone`, `html`, and `sql`.
- Images (`.dat` decryption), voice (SILK → WAV), video lookup, and stickers.
- Every command over HTTP (`serve`), message push, and AI insights.
- A web interface (WebUI) that `serve` hosts under `/ui/` ([details](https://github.com/334456777/WeFlow/wiki/WebUI#网页)).
- `--help`, argument errors, runtime errors, and generated text follow the system language (Chinese or English) by default; `weflow lang en|zh` saves your choice.

## Install

Download the archive for your platform from [Releases](https://github.com/334456777/WeFlow/releases/latest) and unpack it to get `weflow`.

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

The language affects `--help`, argument errors, runtime errors, and generated text. JSON keys and error codes stay in English. For the full precedence order, see the [command reference](https://github.com/334456777/WeFlow/wiki/命令参考#语言).

## First-time setup and export (required steps)

**Run PowerShell as administrator.** WeChat 4.0 or later is supported, and you must have logged in to a WeChat account at least once, sent messages, and opened images.

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

3. Get and save the database key (see details below)

```powershell
weflow key db --save
```

> stdout: `Database key obtained and saved to the active profile`

4. Get and save the image keys (Windows only for now)

```powershell
weflow key image --save
```

> stdout: `Image keys obtained and saved to the active profile`

Without `--save`, the commands print the keys instead of saving them, and you can save them yourself with `weflow config set`.

5. Check that the database opens

```powershell
weflow db test
```

6. Try to list the conversations and export one (a private chat's conversation ID is the other person's wxid; a group's is `xxx@chatroom`)

```powershell
weflow chat sessions
weflow export messages <conversation-id> --format html --out chat.html
```

To export to other formats (`txt`, `json`, `excel`, `sql`, and more), filter by date or sender, or export images and other media along with the messages, see [Message export](https://github.com/334456777/WeFlow/wiki/消息导出#格式) on the Wiki.

You can also run `weflow serve` to open the web interface in your browser.

`weflow config list` shows the saved configuration, and `weflow config path` shows where the configuration file lives.

---

**What does `weflow key db` do?** <br>The database key is generated in memory when WeChat opens its databases. The command extracts `decrypt_key` from that memory location:

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

For the full flow and how it works on each platform, see [Key acquisition](https://github.com/334456777/WeFlow/wiki/密钥获取#数据库密钥) on the Wiki.

## What the CLI does not support

The WeFlow Rust CLI ~~, since my skills aren't up to it,~~ opens databases **read-only** and never modifies WeChat databases ~~— I'm not taking the blame for that~~. Commands that would modify a database (`chat update-message`, `chat delete-message`, `chat anti-revoke`, `chat mark-read`, `sns block-delete`, and `sns delete`) are designed as placeholders. Some features of the WeFlow desktop app are also missing (desktop-process features such as voice-to-text and popups).

See the [detailed list](https://github.com/334456777/WeFlow/wiki/不支持的功能).

## Documentation

[WeFlow Wiki](https://github.com/334456777/WeFlow/wiki)

# Disclaimer

<div id="disclaimer">

## 1. Project purpose and nature

This project (hereinafter “the Project”) was created as a technical research and learning tool for exploring and studying text statistics and analysis techniques. The Project is based on [Attention Is All You Need](https://arxiv.org/abs/1706.03762) and was generated fully automatically by pure [SI](https://www.whitehouse.gov/presidential-actions/2026/09/inaugurating-the-era-of-super-intelligence/), with 0% human content.

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

The developer of the Project (hereinafter “the Developer”) solemnly reminds users to strictly comply with the relevant laws and regulations of the People's Republic of China when downloading, installing, and using the Project, including but not limited to the Cybersecurity Law of the People's Republic of China, the Counter-Espionage Law of the People's Republic of China, and all other applicable national laws and policies. Users bear sole responsibility for any legal liability that may arise from using the Project.

## 3. Restrictions on use

The Project must not be used for any illegal purpose or for any commercial activity other than learning and research. It must not be used to illegally intrude into anyone else's computer system in any form, or to infringe anyone's intellectual-property rights or other lawful rights and interests. Users must ensure that they use the Project purely for personal learning and technical research, and not for illegal activity of any kind.

## 4. Data collection

The Project does not collect, store, or transmit any user data; everything runs locally. Users must ensure that their use of the Project complies with applicable laws and regulations.

## 5. Disclaimer of liability

The Developer has made every effort to ensure the legitimacy and safety of the Project, but accepts no liability for any direct or indirect loss of any kind that may result from its use, including but not limited to data loss, device damage, or legal proceedings.

## 6. Intellectual property

The intellectual property of the WeFlow project belongs to its developer, [hicccc77](https://github.com/hicccc77). The Project is protected by copyright law, international copyright treaties, and other intellectual-property laws and treaties. Users may download and use the Project provided that they comply with this statement and all applicable laws and regulations.

## 7. Final interpretation

The right of final interpretation of the Project belongs to its developer, [hicccc77](https://github.com/hicccc77). The Developer reserves the right to change or update this disclaimer at any time without notice.
</div>
