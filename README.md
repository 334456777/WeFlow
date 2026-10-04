# WeFlow Native CLI

**English** | [简体中文](docs/zh-CN/README.md)

`./weflow` is a native Rust command-line build of [WeFlow](docs/weflow-readme.md)'s backend. It reads, analyzes, and exports local WeChat 4.0+ chat history directly from the terminal, without the Electron desktop app.

- Human-readable output by default (aligned `key: value` fields and tables; errors go to stderr). Add `--json` to emit one JSON document on stdout (`{"success": true, "data": ...}`) for scripting.
- Sessions, messages, contacts, Moments, private/group chat analytics, annual reports, and dual reports.
- Message export in 9 formats: `txt`, `json`, `arkme-json`, `chatlab`, `chatlab-jsonl`, `excel`, `weclone`, `html`, and `sql`.
- Images (`.dat` decryption), voice (SILK → WAV), video lookup, and stickers.
- Local HTTP API (token authentication and SSE push via `serve --http`), message push, and AI insights.
- `--help`, argument errors, runtime errors, and generated text follow the system language (Chinese or English) by default; `./weflow lang en|zh` saves a choice.

> [!WARNING]
> The CLI was ported from the original TypeScript backend. Its database layer is pure Rust (it decrypts and reads WeChat databases itself, read-only) and has been verified with one real Windows WeChat 4.x account (using a Linux build and `weflow.exe` on Windows). WeChat data on macOS/Linux has not been tested ([still to verify](docs/cli-unsupported.md#still-to-verify)). Expect rough edges and please report what you find. See [coverage](docs/cli-coverage.md) and [unsupported features](docs/cli-unsupported.md).

For the original WeFlow project (the Electron desktop app), see [docs/weflow-readme.md](docs/weflow-readme.md).

## Build

```bash
make release
cp target/release/weflow .
./weflow --help
```

## Switching between English and Chinese

```bash
./weflow lang zh
```

The language affects `--help`, argument errors, runtime errors, and generated text. JSON keys and error codes remain in English, as do HTTP API error responses. See [docs/native-cli.md](docs/native-cli.md#language) for the complete precedence order.

## First-time setup and export (required steps)

Run PowerShell **as administrator**. At least one WeChat account must have been logged in on the computer, sent messages, and opened images. WeChat 4.0 and later are supported.

1. Set the WeChat data directory

```powershell
./weflow db detect
./weflow config set db_path "C:\Users\<you>\Documents\xwechat_files"
```

> stdout: db_path: C:\Users\<you>\Documents\xwechat_files

2. Set your wxid

```powershell
./weflow db wxid
./weflow config set wxid wxid_xxxxxxxx
```

> stdout: wxid: wxid_xxxxxxxx

3. Set the database key

```powershell
./weflow key db
./weflow config set decrypt_key <database-key>
```

> stdout: decrypt_key: <database-key>

4. Set the image keys

```powershell
./weflow key image
./weflow config set image_xor_key <image-xor-key>
./weflow config set image_aes_key <image-aes-key>
```

> stdout: image_xor_key: <image-xor-key> <br>
> stdout: image_aes_key: <image-aes-key>

`./weflow config list` shows the saved configuration, and `./weflow config path` shows the configuration file's location.

---

**What does `weflow key db` do?** <br>The database key is generated in memory when WeChat opens a database. This command extracts `decrypt_key` from the corresponding memory location:

```
WeChat is running (pid 31912). Quit it completely first:
system tray icon -> right click -> Quit WeChat
Waiting for WeChat to quit... (exits automatically in 178 s)
WeChat has quit. Open it again.
Waiting for WeChat to start... (exits automatically in 171 s)
WeChat found (pid 20816). Click "Enter WeChat" in the login window.
Waiting for the key... (exits automatically in 150 s)
Database key obtained
decrypt_key: <database-key>
```

The default `180-second automatic exit` can be changed with `./weflow key db --timeout <seconds>`.

## What the CLI does not support

The WeFlow Rust CLI ~~is not skillful enough, so it~~ opens databases **read-only**. It does not modify WeChat databases ~~and some things are not our fault~~. Commands that would modify a database (`chat update-message`, `chat delete-message`, `chat anti-revoke`, `chat mark-read`, `sns block-delete`, and `sns delete`) are retained as placeholders. Some desktop-app features are also missing, including voice-to-text and popups.

See **[docs/cli-unsupported.md](docs/cli-unsupported.md)** for the detailed list ([简体中文](docs/zh-CN/cli-unsupported.md)).

## Documentation

- [Command list](docs/native-cli.md)
- [What the CLI does not support (detailed list)](docs/cli-unsupported.md) · [Coverage of the original backend](docs/cli-coverage.md)
- [Desktop app on the Rust database layer](docs/desktop-rust-layer.md)
- [`wcdb_api.dll`: not needed by the CLI or new desktop app; old and new builds](docs/wcdb-api.md)
- [HTTP API](docs/HTTP-API.md) · [macOS key troubleshooting](docs/MAC-KEY-FAQ.md)
- [Original WeFlow README](docs/weflow-readme.md) · [Español](docs/es-ES/weflow-readme.md)

# Disclaimer

<div id="disclaimer">

## 1. Project purpose and nature

This project (hereinafter “the Project”) was created as a technical research and learning tool to explore and study text statistics and analysis techniques. The Project is based on [Attention Is All You Need](https://arxiv.org/abs/1706.03762) and was generated entirely by pure [SI](https://www.whitehouse.gov/presidential-actions/2026/09/inaugurating-the-era-of-super-intelligence/), with 0% human content.

<table>
  <tr>
    <td><img src="docs/images/SSD.JPG" alt="SSD" width="150"></td>
    <td><img src="docs/images/PMM.JPG" alt="PMM" width="150"></td>
    <td><img src="docs/images/HDD.JPG" alt="HDD" width="150"></td>
    <td><img src="docs/images/MONITOR.JPG" alt="MONITOR" width="150"></td>
  </tr>
  <tr>
    <td><img src="docs/images/MB.JPG" alt="MB" width="150"></td>
    <td><img src="docs/images/GPU.JPG" alt="GPU" width="150"></td>
    <td><img src="docs/images/RAM.JPG" alt="RAM" width="150"></td>
  </tr>
</table>

## 2. Legal compliance statement

The developer of the Project (hereinafter “the Developer”) solemnly reminds users to comply strictly with all applicable laws and policies of the People's Republic of China when downloading, installing, and using the Project, including but not limited to the Cybersecurity Law of the People's Republic of China and the Counter-Espionage Law of the People's Republic of China. Users bear all legal responsibility that may arise from using the Project.

## 3. Restrictions on use

The Project may not be used for any illegal purpose or for commercial activity unrelated to learning or research. It must not be used to intrude illegally into any computer system or infringe another party's intellectual-property rights or other lawful rights and interests. Users must ensure that they use the Project solely for personal learning and technical research and not for any illegal activity.

## 4. Data collected

The Project does not collect, store, or transmit any user data. All operations are performed locally. Users must ensure that their use of the Project complies with applicable laws and regulations.

## 5. Disclaimer of liability

The Developer has made every effort to ensure the legitimacy and security of the Project but accepts no liability for any direct or indirect loss arising from its use, including but not limited to data loss, equipment damage, or legal proceedings.

## 6. Intellectual-property statement

The intellectual property of the WeFlow project belongs to its developer, [hicccc77](https://github.com/hicccc77). The Project is protected by copyright law, international copyright treaties, and other intellectual-property laws and treaties. Users may download and use the Project provided that they comply with this statement and all applicable laws and regulations.

## 7. Final interpretation

The right of final interpretation of the Project belongs to its developer, [hicccc77](https://github.com/hicccc77). The Developer reserves the right to amend or update this disclaimer at any time without prior notice.
</div>
