//! List exports: contacts by kind, every format, the zip with its page and avatars, the sessions named, the
//! footprint.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::common::{account_home, account_home_with, run_cli, start, temp_root, TOKEN};

#[test]
fn contacts_come_parsed_and_export_by_kind() {
    let root = temp_root("contacts");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    // the list the contacts page reads, in process, as the CLI gives it
    let r = server.call("GET", "/api/chat/contacts?parsed=true", "");
    let (_, stdout, _) = run_cli(&home, &["--json", "chat", "contacts", "--parsed"]);
    assert_eq!((r.status, &r.body), (200, &stdout));
    let kinds: Vec<String> = r.json()["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["type"].as_str().unwrap().to_string())
        .collect();
    assert!(
        kinds.iter().any(|k| k == "group") && kinds.iter().any(|k| k == "friend"),
        "{kinds:?}"
    );

    let out = root.join("groups.json");
    let (exit, _, stderr) = run_cli(
        &home,
        &[
            "--json",
            "export",
            "contacts",
            "--parsed",
            "--kind",
            "group",
            "--out",
            &out.to_string_lossy(),
        ],
    );
    assert_eq!(exit, 0, "{stderr}");
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let written = written.as_array().unwrap();
    assert_eq!(
        written.len(),
        kinds.iter().filter(|k| *k == "group").count()
    );
    assert!(written
        .iter()
        .all(|c| c["type"] == "group" && c.get("extra_buffer").is_none()));

    // a kind asks for the parsed list
    let (exit, _, _) = run_cli(
        &home,
        &[
            "export",
            "contacts",
            "--kind",
            "group",
            "--out",
            &out.to_string_lossy(),
        ],
    );
    assert_eq!(exit, 2);
}

/// The lists as tables (#659): a row for each conversation or contact, and the formats for messages refused.
#[test]
fn list_exports_write_the_rows_in_every_format() {
    use std::io::Read;
    let root = temp_root("list-exports");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let out = root.join("out");
    for (route, more) in [
        ("/export/sessions", json!({})),
        ("/export/contacts", json!({})),
        ("/export/contacts", json!({ "parsed": true })),
    ] {
        for format in ["html", "sql", "excel", "csv", "json"] {
            let mut body = more.clone();
            body["format"] = json!(format);
            body["out"] = json!(out);
            let r = server.call("POST", route, &body.to_string());
            assert_eq!(r.status, 200, "{route} as {format}: {}", r.body);
            let path = PathBuf::from(r.json()["data"]["out"].as_str().unwrap());
            let text = if format == "excel" {
                // the strings of a workbook are in one of its XML parts
                let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
                let mut all = String::new();
                for i in 0..zip.len() {
                    let _ = zip.by_index(i).unwrap().read_to_string(&mut all);
                }
                all
            } else {
                std::fs::read_to_string(&path).unwrap()
            };
            // a CSV starts with the byte order mark, which Excel needs to read it as UTF-8 (#1075)
            assert_eq!(
                text.starts_with('\u{feff}'),
                format == "csv",
                "{route} as {format} ({more})"
            );
            assert!(
                text.contains("wxid_bob"),
                "{route} as {format} ({more}) has no row for wxid_bob"
            );
        }
        for format in ["chatlab", "weclone"] {
            let mut body = more.clone();
            body["format"] = json!(format);
            body["out"] = json!(out.join("refused"));
            let r = server.call("POST", route, &body.to_string());
            assert_eq!(r.status, 400, "{route} as {format}: {}", r.body);
            assert_eq!(r.json()["error"]["code"], "invalid_arguments");
        }
    }
    assert!(!out.join("refused").exists());
}

/// `--zip` (#708): the page of a list and its avatars in one zip, which is the file the export names, so the WebUI
/// downloads both.
#[test]
fn a_list_export_packs_its_page_and_avatars_into_a_zip() {
    use std::io::Read;
    let root = temp_root("list-zip");
    let picture = [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4];
    let home = account_home_with(&root, |f| f.head_image_db(&[("wxid_bob", &picture)]));
    let server = start(&home, &["--api-token", TOKEN]);
    let out = root.join("out");
    let body = json!({
        "format": "html", "parsed": true, "media": ["avatar"], "no-avatar-download": true, "zip": true, "out": out,
    });
    let r = server.call("POST", "/export/contacts", &body.to_string());
    assert_eq!(r.status, 200, "{}", r.body);
    let path = PathBuf::from(r.json()["data"]["out"].as_str().unwrap());
    assert_eq!(path, out.join("contacts.zip"));
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let avatar = names
        .iter()
        .find(|n| n.starts_with("media/contacts/avatars/"))
        .unwrap_or_else(|| panic!("no avatar in {names:?}"))
        .clone();
    let mut page = String::new();
    zip.by_name("contacts.html")
        .unwrap()
        .read_to_string(&mut page)
        .unwrap();
    assert!(page.contains(&avatar), "the page points at {avatar}");
    let mut bytes = Vec::new();
    zip.by_name(&avatar)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, picture);
    // only the zip is left beside it
    let left: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, ["contacts.zip"]);

    // a zip is not written under another file's name
    let body = json!({ "format": "html", "zip": true, "out": out.join("contacts.html") });
    let r = server.call("POST", "/export/contacts", &body.to_string());
    assert_eq!(r.status, 400, "{}", r.body);
}

#[test]
fn a_session_list_export_keeps_only_the_sessions_named() {
    let root = temp_root("sessions-named");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let out = root.join("named.json");
    let body = json!({ "session": ["wxid_bob"], "out": out });
    let r = server.call("POST", "/export/sessions", &body.to_string());
    assert_eq!(r.status, 200, "{}", r.body);
    let rows: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let names: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["username"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["wxid_bob"]);
    // a part of an ID, or a key of the rows, names no session (#664)
    for session in ["wxid_bo", "username"] {
        let none = root.join(format!("{session}.json"));
        let body = json!({ "session": [session], "out": none });
        let r = server.call("POST", "/export/sessions", &body.to_string());
        assert_eq!(r.status, 400, "{session}: {}", r.body);
        assert_eq!(r.json()["error"]["code"], "invalid_arguments");
        assert!(!none.exists(), "{session}");
    }
}

/// `export footprint` (#315): JSON as it is, CSV and Excel as the timeline; the formats for messages refused before
/// anything is read.
#[test]
fn the_footprint_is_exported_as_json_or_as_a_timeline() {
    let root = temp_root("footprint");
    let home = account_home(&root);
    let export = |format: &str| {
        let out = root.join("out").join(format);
        let (code, stdout, stderr) = run_cli(
            &home,
            &[
                "--json",
                "export",
                "footprint",
                "--format",
                format,
                "--out",
                out.to_str().unwrap(),
            ],
        );
        (
            code,
            serde_json::from_str::<Value>(&stdout).unwrap_or(Value::Null),
            stderr,
        )
    };

    // CSV: a row for each of the four segments with Bob, by the name shown for him (his remark)
    let (code, v, stderr) = export("csv");
    assert_eq!(code, 0, "{stderr}");
    let csv = std::fs::read_to_string(v["data"]["out"].as_str().unwrap()).unwrap();
    // the byte order mark Excel needs to read it as UTF-8 (#1075), then the table
    let csv = csv
        .strip_prefix('\u{feff}')
        .unwrap_or_else(|| panic!("no byte order mark: {csv}"));
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines[0],
        "Time,Kind,Conversation,Sender,Received,Sent,Replied,Content,Conversation ID"
    );
    assert_eq!(lines.len(), 5, "{csv}");
    assert!(
        lines[1..]
            .iter()
            .all(|l| l.contains(",private chat,Bobby,,") && l.ends_with(",wxid_bob")),
        "{csv}"
    );
    assert!(lines[1].contains(",1,1,yes,"), "{csv}");

    // Excel: the same table in a workbook
    let (code, v, stderr) = export("excel");
    assert_eq!(code, 0, "{stderr}");
    let xlsx = std::fs::read(v["data"]["out"].as_str().unwrap()).unwrap();
    assert!(xlsx.starts_with(b"PK"), "an xlsx is a zip");

    // JSON as it is
    let (code, v, _) = export("json");
    assert_eq!(code, 0);
    let json: Value =
        serde_json::from_str(&std::fs::read_to_string(v["data"]["out"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(json["private_segments"].as_array().map(Vec::len), Some(4));

    // a format for messages: refused, nothing written
    let (code, v, _) = export("html");
    assert_eq!(code, 2);
    assert_eq!(v["error"]["code"], "invalid_arguments");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("json, csv or excel"),
        "{v}"
    );
    assert!(!root.join("out").join("html").exists());
}

/// `export messages --zip --media` (#1039): the result names the media folder as it is inside the zip, not the folder
/// the zip was made from, which is taken away; every format's file and the result's `media` say the same folder.
#[test]
fn a_messages_zip_names_its_media_folder_inside_the_zip() {
    use std::io::Read;
    use weflow_native::fixture::{DAY, T0};
    let root = temp_root("messages-zip-media");
    // a stand-in JPEG: SOI, bytes, EOI (without its end it would be taken for one cut short)
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
    jpeg.extend((0..3000u32).map(|i| (i as u8).wrapping_mul(7) | 1));
    jpeg.extend([0xFF, 0xD9]);
    let home = account_home_with(&root, |f| {
        // the image of `wxid_bob`'s message 3, an old-style .dat: the whole file XOR-ed
        let img = f
            .account_dir
            .join("msg/attach")
            .join(weflow_core::image::session_dir_for_storage("wxid_bob"))
            .join(weflow_core::image::year_month_from_create_time(Some(
                T0 + DAY,
            )))
            .join("Img");
        std::fs::create_dir_all(&img).unwrap();
        std::fs::write(
            img.join("aabbccddeeff00112233445566778899.dat"),
            jpeg.iter().map(|b| b ^ 0x5a).collect::<Vec<u8>>(),
        )
        .unwrap();
    });
    let (exit, _, stderr) = run_cli(&home, &["config", "set", "image_xor_key", "90"]);
    assert_eq!(exit, 0, "{stderr}");

    let out = root.join("out").join("bob.zip");
    let (exit, stdout, stderr) = run_cli(
        &home,
        &[
            "--json",
            "export",
            "messages",
            "wxid_bob",
            "--out",
            out.to_str().unwrap(),
            "--format",
            "json,txt",
            "--zip",
            "--media",
            "image",
        ],
    );
    assert_eq!(exit, 0, "{stdout}{stderr}");
    let result: Value = serde_json::from_str(&stdout).unwrap();
    let data = &result["data"];
    assert_eq!(data["media"]["exported"], 1, "{data}");

    let dir = data["media"]["dir"].as_str().unwrap().to_string();
    assert!(!dir.contains(".parts"), "{dir}");
    assert_eq!(dir, "media/wxid_bob");
    let files = data["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{data}");
    assert!(
        files.iter().all(|f| f["media"]["dir"] == dir.as_str()),
        "{data}"
    );

    // the folder is in the zip, with the image in it; the one the zip was made from is gone
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let inside = format!("{dir}/");
    assert!(
        names.iter().any(|n| n.starts_with(&inside)),
        "no file under {dir} in {names:?}"
    );
    assert!(names.contains(&"wxid_bob.json".to_string()), "{names:?}");
    assert!(names.contains(&"wxid_bob.txt".to_string()), "{names:?}");
    let left: Vec<_> = std::fs::read_dir(out.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, ["bob.zip"]);

    // the messages point at the same folder as the result does
    let mut messages = String::new();
    zip.by_name("wxid_bob.json")
        .unwrap()
        .read_to_string(&mut messages)
        .unwrap();
    assert!(messages.contains(&inside), "{messages}");
}

/// `export messages --zip` (#1040) writes a zip, so `--out` is a folder or a `.zip` file, as for the lists: not a
/// file with another extension, whose name would claim a text, a page or a table for a zip.
#[test]
fn a_messages_zip_is_not_written_under_another_files_name() {
    let root = temp_root("messages-zip-name");
    let home = account_home(&root);
    let out = root.join("out");
    let export = |target: &Path, formats: &str| {
        run_cli(
            &home,
            &[
                "--json",
                "export",
                "messages",
                "wxid_bob",
                "--out",
                target.to_str().unwrap(),
                "--format",
                formats,
                "--zip",
            ],
        )
    };

    for (name, formats) in [
        ("chat.txt", "txt"),
        ("chat.html", "html,txt"),
        ("chat.json", "json"),
    ] {
        let target = out.join(name);
        let (exit, stdout, _) = export(&target, formats);
        assert_eq!(exit, 2, "{name}: {stdout}");
        let v: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(v["error"]["code"], "invalid_arguments", "{name}: {v}");
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("--zip writes a zip"),
            "{name}: {v}"
        );
        // nothing was written, not even the folder the zip is made in
        assert!(!out.exists(), "{name}");
    }

    // a .zip file, in any case, and a folder (the zip goes in it, named after the conversation)
    for (target, zipped) in [
        (
            out.join("a").join("chat.zip"),
            out.join("a").join("chat.zip"),
        ),
        (
            out.join("b").join("CHAT.ZIP"),
            out.join("b").join("CHAT.ZIP"),
        ),
        (out.join("c"), out.join("c").join("wxid_bob.zip")),
    ] {
        let (exit, stdout, stderr) = export(&target, "txt");
        assert_eq!(exit, 0, "{}: {stdout}{stderr}", target.display());
        let v: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(PathBuf::from(v["data"]["out"].as_str().unwrap()), zipped);
        let zip = zip::ZipArchive::new(std::fs::File::open(&zipped).unwrap()).unwrap();
        assert_eq!(zip.file_names().collect::<Vec<_>>(), ["wxid_bob.txt"]);
    }

    // the same through `serve`
    let server = start(&home, &["--api-token", TOKEN]);
    let body = json!({ "session-id": "wxid_bob", "zip": true, "out": root.join("serve").join("chat.txt") });
    let r = server.call("POST", "/export/messages", &body.to_string());
    assert_eq!(r.status, 400, "{}", r.body);
    assert_eq!(r.json()["error"]["code"], "invalid_arguments");
    assert!(!root.join("serve").exists());
}

/// `--format ""` stands for the default format, txt, both on the command line and through `serve`.
#[test]
fn an_empty_message_format_exports_txt() {
    let root = temp_root("empty-format");
    let home = account_home(&root);
    let out = root.join("cli");
    let (code, stdout, stderr) = run_cli(
        &home,
        &[
            "--json",
            "export",
            "messages",
            "wxid_bob",
            "--format",
            "",
            "--out",
            out.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let text = std::fs::read_to_string(out.join("wxid_bob.txt")).unwrap();
    assert!(text.contains("see you"), "{text}");

    let server = start(&home, &["--api-token", TOKEN]);
    let out = root.join("serve");
    let body = json!({ "session-id": "wxid_bob", "format": ["", " "], "out": out });
    let r = server.call("POST", "/export/messages", &body.to_string());
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(out.join("wxid_bob.txt").is_file());
}
