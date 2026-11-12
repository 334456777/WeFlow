//! List exports: contacts by kind, every format, the zip with its page and avatars, the sessions named, the
//! footprint.

use std::path::PathBuf;

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
