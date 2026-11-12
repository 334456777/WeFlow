//! The answers of each kind of command held against the contract (`web/contract/openapi.json`).

use std::io::{BufRead, BufReader, Write};

use serde_json::{json, Value};

use super::common::{
    account_home, account_home_with, contract, misfits, percent, run_cli, start, temp_root, TOKEN,
};

#[test]
fn reports_fit_the_contract() {
    let root = temp_root("reports");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    // the year of the account's messages, and all the years
    for (route, body, schema) in [
        (
            "/report/annual/generate",
            r#"{"year": 2023}"#,
            "AnnualReport",
        ),
        ("/report/annual/generate", r#"{"year": 0}"#, "AnnualReport"),
        (
            "/report/dual/generate",
            r#"{"friend": "wxid_bob", "year": 2023}"#,
            "DualReport",
        ),
        (
            "/report/dual/generate",
            r#"{"friend": "wxid_bob", "year": 0}"#,
            "DualReport",
        ),
    ] {
        let r = server.call("POST", route, body);
        assert_eq!(r.status, 200, "{route} {body}: {}", r.body);
        let data = &r.json()["data"];
        let fits = misfits(
            &contract,
            &json!({ "$ref": format!("#/components/schemas/{schema}") }),
            data,
        );
        assert!(fits.is_empty(), "{route} {body}: {fits:#?}\n{data}");
        assert!(data["year"].is_i64(), "{data}");
    }
    let dual = server.call(
        "POST",
        "/report/dual/generate",
        r#"{"friend": "wxid_bob", "year": 2023}"#,
    );
    let dual = &dual.json()["data"];
    assert!(dual["yearFirstChat"].is_object(), "{dual}");
    assert!(dual["firstChat"]["senderUsername"].is_string(), "{dual}");
}

#[test]
fn file_exports_fit_the_contract() {
    let root = temp_root("exports-contract");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    let out = root.join("out");
    for (route, body, schema) in [
        (
            "/export/sessions",
            json!({ "out": out.join("s.json") }),
            "ExportWritten",
        ),
        (
            "/export/contacts",
            json!({ "out": out.join("c.csv"), "format": "csv" }),
            "ExportWritten",
        ),
        (
            "/export/footprint",
            json!({ "out": out.join("f.json") }),
            "ExportWritten",
        ),
        (
            "/group/export-members",
            json!({ "chatroom-id": "room1@chatroom", "out": out.join("m.csv") }),
            "GroupExported",
        ),
        (
            "/group/export-member-messages",
            json!({ "chatroom-id": "room1@chatroom", "username": "wxid_bob", "out": out.join("b.csv") }),
            "GroupExported",
        ),
        (
            "/sns/export",
            json!({ "out": out.join("sns") }),
            "SnsExported",
        ),
    ] {
        let r = server.call("POST", route, &body.to_string());
        assert_eq!(r.status, 200, "{route}: {}", r.body);
        let data = &r.json()["data"];
        let misfit = misfits(
            &contract,
            &json!({ "$ref": format!("#/components/schemas/{schema}") }),
            data,
        );
        assert!(misfit.is_empty(), "{route}: {misfit:#?}\n{data}");
    }
}

#[test]
fn chat_media_answers_fit_the_contract() {
    use weflow_native::fixture::{DAY, T0};
    let root = temp_root("chat-media-contract");
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
    jpeg.extend((0..3000u32).map(|i| (i as u8).wrapping_mul(7) | 1));
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
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    let image_out = root.join("out").join("image.jpg");
    let preload = json!({
        "session-id": "wxid_bob",
        "messages-json": json!([{ "localId": 9, "createTime": T0, "serverId": "77" }]).to_string(),
    });
    for (verb, route, body, schema) in [
        ("GET", "/chat/images?session-id=wxid_bob".to_string(), String::new(), "ChatImages"),
        (
            "GET",
            "/chat/voice-messages?session-id=wxid_bob".into(),
            String::new(),
            "VoiceMessages",
        ),
        ("GET", "/chat/media-stream?limit=10".into(), String::new(), "MediaStream"),
        (
            "GET",
            "/chat/voice-cache?session-id=wxid_bob&msg-id=5".into(),
            String::new(),
            "VoiceCache",
        ),
        ("POST", "/chat/voice-preload".into(), preload.to_string(), "VoicesPrepared"),
        (
            "POST",
            "/chat/emoji".into(),
            json!({ "session-id": "wxid_bob", "out": root.join("emoji") }).to_string(),
            "EmojisDownloaded",
        ),
        (
            "GET",
            "/chat/group-hint?chatroom-id=room1%40chatroom".into(),
            String::new(),
            "GroupHint",
        ),
        (
            "PUT",
            "/chat/group-hint".into(),
            json!({ "chatroom-id": "room1@chatroom", "set": 4 }).to_string(),
            "GroupHint",
        ),
        (
            "GET",
            "/chat/group-hint?chatroom-id=room1%40chatroom".into(),
            String::new(),
            "GroupHint",
        ),
        ("GET", "/chat/anti-revoke/check".into(), String::new(), "AntiRevokeResults"),
        (
            "POST",
            "/chat/image-data".into(),
            json!({ "session-id": "wxid_bob", "msg-id": "3", "create-time": T0 + DAY, "out": image_out })
                .to_string(),
            "MediaWritten",
        ),
    ] {
        let r = server.call(verb, &route, &body);
        assert_eq!(r.status, 200, "{verb} {route} {body}: {}", r.body);
        let data = &r.json()["data"];
        let misfit = misfits(
            &contract,
            &json!({ "$ref": format!("#/components/schemas/{schema}") }),
            data,
        );
        assert!(misfit.is_empty(), "{route}: {misfit:#?}\n{data}");
    }
    let sessions = server.call("GET", "/chat/anti-revoke/sessions", "");
    assert_eq!(sessions.status, 200, "{}", sessions.body);
    let misfit = misfits(
        &contract,
        &json!({ "type": "array", "items": { "$ref": "#/components/schemas/Row" } }),
        &sessions.json()["data"],
    );
    assert!(misfit.is_empty(), "{misfit:#?}");
}

/// A media server that answers every request with `body` as a PNG, with the headers WeChat's servers send.
fn png_server(body: Vec<u8>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut request = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while request.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nx-enc: 1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    url
}

#[test]
fn sns_downloads_fit_the_contract() {
    let root = temp_root("sns-downloads-contract");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend((0..200u8).map(|i| i | 1));
    let base = png_server(png);
    let out = root.join("out").join("image.png");
    let debug = format!(
        "/sns/debug-resource?url={}",
        percent(&format!("{base}/res"))
    );
    for (verb, route, body, schema) in [
        ("GET", debug.as_str(), String::new(), "SnsResourceProbe"),
        (
            "POST",
            "/sns/download-emoji",
            json!({ "url": format!("{base}/emoji") }).to_string(),
            "SnsEmojiSaved",
        ),
        (
            "POST",
            "/sns/download-image",
            json!({ "url": format!("{base}/a") }).to_string(),
            "SnsImageInline",
        ),
        (
            "POST",
            "/sns/download-image",
            json!({ "url": format!("{base}/b"), "out": out }).to_string(),
            "SnsImageSaved",
        ),
    ] {
        let r = server.call(verb, route, &body);
        assert_eq!(r.status, 200, "{verb} {route} {body}: {}", r.body);
        let data = &r.json()["data"];
        let misfit = misfits(
            &contract,
            &json!({ "$ref": format!("#/components/schemas/{schema}") }),
            data,
        );
        assert!(misfit.is_empty(), "{route}: {misfit:#?}\n{data}");
    }
    assert!(out.is_file());
}

#[test]
fn db_scan_and_open_fit_the_contract() {
    let root = temp_root("db-contract");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    // the folder the fixture account is in; `key db` and `key image` need WeChat itself, and are left out
    let scan = server.call(
        "GET",
        &format!(
            "/db/scan?root={}",
            percent(&root.join("data").to_string_lossy())
        ),
        "",
    );
    assert_eq!(scan.status, 200, "{}", scan.body);
    let data = &scan.json()["data"];
    let misfit = misfits(
        &contract,
        &json!({ "$ref": "#/components/schemas/DbScan" }),
        data,
    );
    assert!(misfit.is_empty(), "{misfit:#?}\n{data}");
    assert_eq!(data["accounts"].as_array().unwrap().len(), 1, "{data}");
    let open = server.call("GET", "/db/open", "");
    assert_eq!(open.status, 200, "{}", open.body);
    let misfit = misfits(
        &contract,
        &json!({ "$ref": "#/components/schemas/DbConnection" }),
        &open.json()["data"],
    );
    assert!(misfit.is_empty(), "{misfit:#?}");
}

#[test]
fn tool_answers_fit_the_contract() {
    let root = temp_root("tools-contract");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    // what needs no network: the downloads (`web install`, `use`, `update`, `list --remote`) are left out
    for (verb, route, body, schema) in [
        ("GET", "/runtime/manifest", "", "RuntimeManifest"),
        ("GET", "/web/info", "", "WebInfo"),
        ("DELETE", "/web/prune", "", "WebPruned"),
    ] {
        let r = server.call(verb, route, body);
        assert_eq!(r.status, 200, "{verb} {route} {body}: {}", r.body);
        let data = &r.json()["data"];
        let misfit = misfits(
            &contract,
            &json!({ "$ref": format!("#/components/schemas/{schema}") }),
            data,
        );
        assert!(misfit.is_empty(), "{route}: {misfit:#?}\n{data}");
    }
    let r = server.call("GET", "/web/list", "");
    assert_eq!(r.status, 200, "{}", r.body);
    let misfit = misfits(
        &contract,
        &json!({ "type": "array", "items": { "$ref": "#/components/schemas/WebInstalled" } }),
        &r.json()["data"],
    );
    assert!(misfit.is_empty(), "{misfit:#?}");
}

#[test]
fn image_and_video_answers_fit_the_contract() {
    use weflow_native::fixture::{DAY, T0};
    let root = temp_root("image-contract");
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
    jpeg.extend((0..3000u32).map(|i| (i as u8).wrapping_mul(7) | 1));
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
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    let image = json!({
        "session": "wxid_bob",
        "md5": "aabbccddeeff00112233445566778899",
        "create-time": T0 + DAY,
        "prefer-file-path": true,
    });
    let payload = json!({
        "sessionId": "wxid_bob",
        "imageMd5": "aabbccddeeff00112233445566778899",
        "createTime": T0 + DAY,
    });
    let resolve = format!(
        "/image/resolve-cache?session=wxid_bob&md5=aabbccddeeff00112233445566778899&create-time={}&prefer-file-path=true",
        T0 + DAY
    );
    let parse = format!(
        "/video/parse-md5?content={}",
        percent("<msg><videomsg md5=\"abc\"/></msg>")
    );
    for (verb, route, body, schema) in [
        ("POST", "/image/decrypt", image.to_string(), "ImageResult"),
        ("GET", resolve.as_str(), String::new(), "ImageResult"),
        (
            "POST",
            "/image/resolve-batch",
            json!({ "payloads-json": json!([payload, payload]).to_string() }).to_string(),
            "ImageBatch",
        ),
        (
            "GET",
            "/image/auto-download/status",
            String::new(),
            "AutoDownloadStatus",
        ),
        (
            "GET",
            &format!("/video/info?md5={}", "0".repeat(32)),
            String::new(),
            "VideoInfo",
        ),
        ("GET", parse.as_str(), String::new(), "VideoMd5"),
    ] {
        let r = server.call(verb, route, &body);
        assert_eq!(r.status, 200, "{verb} {route} {body}: {}", r.body);
        let data = &r.json()["data"];
        let misfit = misfits(
            &contract,
            &json!({ "$ref": format!("#/components/schemas/{schema}") }),
            data,
        );
        assert!(misfit.is_empty(), "{route}: {misfit:#?}\n{data}");
    }
}

#[test]
fn message_and_media_exports_fit_the_contract() {
    let root = temp_root("message-exports-contract");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    let out = root.join("out");
    for (route, body, schema) in [
        (
            "/export/messages",
            json!({ "session-id": "wxid_bob", "out": out.join("bob.json"), "format": "json" }),
            "MessagesExported",
        ),
        (
            "/export/messages",
            json!({ "session-id": "wxid_bob", "out": out.join("media.json"), "format": "json", "media": "image" }),
            "MessagesExported",
        ),
        (
            "/export/messages",
            json!({ "session-id": "wxid_bob", "out": out.join("both"), "format": ["json", "txt"] }),
            "MessagesBundle",
        ),
        (
            "/export/messages",
            json!({ "session-id": "wxid_bob", "out": out.join("packed"), "format": ["json", "txt"], "zip": true }),
            "MessagesBundle",
        ),
        (
            "/export/media",
            json!({ "out": out.join("media") }),
            "MediaExport",
        ),
        (
            "/chat/voice",
            json!({ "session-id": "wxid_bob", "out": out.join("voice") }),
            "MediaExport",
        ),
    ] {
        let r = server.call("POST", route, &body.to_string());
        assert_eq!(r.status, 200, "{route} {body}: {}", r.body);
        let data = &r.json()["data"];
        let misfit = misfits(
            &contract,
            &json!({ "$ref": format!("#/components/schemas/{schema}") }),
            data,
        );
        assert!(misfit.is_empty(), "{route} {body}: {misfit:#?}\n{data}");
    }
}

#[test]
fn backup_answers_fit_the_contract() {
    let root = temp_root("backup-contract");
    let home = account_home_with(&root, |f| {
        let video = f.account_dir.join("msg/video/2023-11");
        std::fs::create_dir_all(&video).unwrap();
        std::fs::write(video.join("v.mp4"), b"video").unwrap();
    });
    // a sticker WeFlow downloaded, in its sticker cache
    let cache = root.join("cache");
    std::fs::create_dir_all(cache.join("Emojis")).unwrap();
    std::fs::write(cache.join("Emojis/e.gif"), b"sticker").unwrap();
    let (exit, _, stderr) = run_cli(
        &home,
        &["config", "set", "cache_path", cache.to_str().unwrap()],
    );
    assert_eq!(exit, 0, "{stderr}");
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    let fits = |verb: &str, route: &str, body: String, schema: &str| -> Value {
        let r = server.call(verb, route, &body);
        assert_eq!(r.status, 200, "{verb} {route} {body}: {}", r.body);
        let data = r.json()["data"].clone();
        let misfit = misfits(
            &contract,
            &json!({ "$ref": format!("#/components/schemas/{schema}") }),
            &data,
        );
        assert!(misfit.is_empty(), "{route}: {misfit:#?}\n{data}");
        data
    };
    let out = root.join("backup.zip");
    let created = fits(
        "POST",
        "/backup/create",
        json!({ "out": out }).to_string(),
        "BackupCreated",
    );
    let manifest = fits(
        "GET",
        &format!("/backup/inspect?path={}", percent(&out.to_string_lossy())),
        String::new(),
        "BackupManifest",
    );
    assert_eq!(
        manifest["entries"].as_array().unwrap().len() as i64,
        created["entries"].as_i64().unwrap(),
        "{manifest}"
    );
    // the databases only, unless more is asked for (#643)
    let paths = |manifest: &Value| -> Vec<String> {
        manifest["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["path"].as_str().unwrap().to_string())
            .collect()
    };
    assert!(
        paths(&manifest)
            .iter()
            .all(|p| p.starts_with("db_storage/")),
        "{manifest}"
    );
    let restored = fits(
        "POST",
        "/backup/restore",
        json!({ "path": out, "target": root.join("restored") }).to_string(),
        "BackupRestored",
    );
    assert_eq!(restored.get("stickersRestoredTo"), None, "{restored}");

    let more = root.join("more.zip");
    fits(
        "POST",
        "/backup/create",
        json!({ "out": more, "videos": true, "emojis": true }).to_string(),
        "BackupCreated",
    );
    let manifest = fits(
        "GET",
        &format!("/backup/inspect?path={}", percent(&more.to_string_lossy())),
        String::new(),
        "BackupManifest",
    );
    let paths = paths(&manifest);
    for wanted in ["msg/video/2023-11/v.mp4", "emojis/e.gif"] {
        assert!(paths.iter().any(|p| p == wanted), "{wanted}: {paths:?}");
    }
    std::fs::remove_file(cache.join("Emojis/e.gif")).unwrap();
    let restored = fits(
        "POST",
        "/backup/restore",
        json!({ "path": more, "target": root.join("restored-more") }).to_string(),
        "BackupRestored",
    );
    assert_eq!(
        std::fs::read(root.join("restored-more/msg/video/2023-11/v.mp4")).unwrap(),
        b"video"
    );
    // the sticker went back into the sticker cache
    assert!(restored["stickersRestoredTo"].is_string(), "{restored}");
    assert_eq!(
        std::fs::read(cache.join("Emojis/e.gif")).unwrap(),
        b"sticker"
    );
    // the options of older WeFlows are gone
    let r = server.call(
        "POST",
        "/backup/create",
        &json!({ "out": root.join("old.zip"), "no-images": true }).to_string(),
    );
    assert_eq!(r.status, 400, "{}", r.body);
}

#[test]
fn settings_changes_fit_the_contract() {
    let root = temp_root("settings-contract");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    let fits = |verb: &str, route: &str, body: &str, schema: Value| -> Value {
        let r = server.call(verb, route, body);
        assert_eq!(r.status, 200, "{verb} {route} {body}: {}", r.body);
        let data = r.json()["data"].clone();
        let misfit = misfits(&contract, &schema, &data);
        assert!(misfit.is_empty(), "{route} {body}: {misfit:#?}\n{data}");
        data
    };
    let component = |name: &str| json!({ "$ref": format!("#/components/schemas/{name}") });
    let plan = fits(
        "DELETE",
        "/cache/clear",
        r#"{"images": true, "dry-run": true}"#,
        component("RemovalPlan"),
    );
    assert_eq!(plan["dryRun"], true, "{plan}");
    fits(
        "DELETE",
        "/cache/clear",
        r#"{"images": true, "yes": true}"#,
        component("CacheCleared"),
    );
    fits(
        "DELETE",
        "/cache/clear-account",
        r#"{"dry-run": true}"#,
        component("RemovalPlan"),
    );
    fits(
        "PUT",
        "/config/set",
        r#"{"key": "progress_delay_seconds", "value": "0"}"#,
        component("ConfigWritten"),
    );
    fits(
        "DELETE",
        "/config/unset",
        r#"{"key": "progress_delay_seconds"}"#,
        component("ConfigWritten"),
    );
    fits("GET", "/config/list", "", component("ConfigStore"));
    fits("GET", "/config/path", "", json!({ "type": "string" }));
    fits("PUT", "/lang", r#"{"lang": "en"}"#, component("LangSaved"));
    // last: the account goes with the rest of the config
    fits("DELETE", "/config/clear", "", component("ConfigWritten"));
}

#[test]
fn chat_stats_fit_the_contract() {
    let root = temp_root("chat-stats-contract");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    let check = |route: &str, schema: &str| -> Value {
        let r = server.call("GET", route, "");
        assert_eq!(r.status, 200, "{route}: {}", r.body);
        let data = r.json()["data"].clone();
        let fits = misfits(
            &contract,
            &json!({ "$ref": format!("#/components/schemas/{schema}") }),
            &data,
        );
        assert!(fits.is_empty(), "{route}: {fits:#?}\n{data}");
        data
    };
    let stats = check(
        "/chat/export-stats?sessions=wxid_bob&sessions=room1%40chatroom",
        "ExportStats",
    );
    // a private chat's relations and a group's, in the order asked for
    let ids: Vec<&String> = stats["data"].as_object().unwrap().keys().collect();
    assert_eq!(ids, ["wxid_bob", "room1@chatroom"]);
    assert!(
        stats["data"]["wxid_bob"]["privateMutualGroups"].is_i64(),
        "{stats}"
    );
    assert!(
        stats["data"]["room1@chatroom"]["groupMemberCount"].is_i64(),
        "{stats}"
    );
    check(
        "/chat/export-stats?sessions=wxid_bob&no-relations=true",
        "ExportStats",
    );
    check(
        "/chat/transfer-names?chatroom-id=room1%40chatroom&payer=wxid_bob&receiver=wxid_me",
        "TransferNames",
    );
}

#[test]
fn insight_answers_fit_the_contract() {
    let root = temp_root("insight-contract");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    // without an AI set up: no records, and a scan that writes none
    for (verb, route, schema) in [
        (
            "GET",
            "/insight/records?limit=10",
            json!({ "$ref": "#/components/schemas/InsightRecords" }),
        ),
        (
            "GET",
            "/insight/today-stats",
            json!({ "type": "array", "items": { "$ref": "#/components/schemas/InsightTodayStat" } }),
        ),
        (
            "DELETE",
            "/insight/clear",
            json!({ "$ref": "#/components/schemas/InsightCleared" }),
        ),
        (
            "POST",
            "/insight/scan",
            json!({ "$ref": "#/components/schemas/InsightScan" }),
        ),
    ] {
        let r = server.call(verb, route, "");
        assert_eq!(r.status, 200, "{verb} {route}: {}", r.body);
        let data = &r.json()["data"];
        let fits = misfits(&contract, &schema, data);
        assert!(fits.is_empty(), "{route}: {fits:#?}\n{data}");
    }
    // a record that is not there is the command's failure, as the contract leaves it
    let r = server.call("PUT", "/insight/mark-read", r#"{"id": "nothing-here"}"#);
    assert_ne!(r.status, 200, "{}", r.body);
}

/// The answers of the routes the contract describes fit it as the server sends them, from its own process and from a
/// child, over an account whose messages lie in three databases with keys that tie; pages walked from their cursors
/// list each message once; failures fit `Failure` and jobs `Job` (#400).
#[test]
fn answers_fit_the_contract() {
    use weflow_native::fixture::{MsgSpec, DAY, T0};
    let root = temp_root("contract");
    let home = account_home_with(&root, |f| {
        f.message_shard(
            2,
            &[(
                "wxid_bob",
                vec![
                    // the very time of the second database's message 5
                    MsgSpec::text(6, "wxid_bob", T0 + 4 * DAY, "same time"),
                    MsgSpec::text(
                        7,
                        "wxid_me",
                        T0 + 5 * DAY,
                        "<msg><appmsg><type>6</type><title>plan.pdf</title><appattach><totallen>2048</totallen>\
                         <fileext>pdf</fileext></appattach></appmsg></msg>",
                    )
                    .of_type(49),
                    // a row that does not say who sent it
                    MsgSpec::text(8, "", T0 + 5 * DAY + 1, "\"Bob\" recalled a message")
                        .of_type(10000),
                ],
            )],
        );
    });
    let server = start(&home, &["--api-token", TOKEN]);
    let contract = contract();
    let component = |name: &str| json!({ "$ref": format!("#/components/schemas/{name}") });
    let list_of = |name: &str| json!({ "type": "array", "items": component(name) });
    let answer_schema = |route: &str, status: &str| {
        let schema = contract["paths"][route]["get"]["responses"][status]["content"]
            ["application/json"]["schema"]
            .clone();
        assert!(
            schema.is_object(),
            "{route} {status} is not in the contract"
        );
        schema
    };
    let check = |what: &str, schema: &Value, value: &Value| {
        let errors = misfits(&contract, schema, value);
        assert!(errors.is_empty(), "{what}: {errors:#?}\n{value}");
    };
    // the `data` of `target`, in the process and from a child, both fitting the route's answer and, where the route
    // answers one of several shapes, `shape`: a parsed message fits `Row` too
    let read = |target: &str, shape: Option<Value>| -> Value {
        let route = target.split('?').next().unwrap();
        let ok = answer_schema(route, "200");
        let own = server.call("GET", target, "");
        let child = server.call("GET", target.strip_prefix("/api").unwrap(), "");
        for r in [&own, &child] {
            assert_eq!(r.status, 200, "{target}: {}", r.body);
            check(target, &ok, &r.json());
            if let Some(shape) = &shape {
                check(target, shape, &r.json()["data"]);
            }
        }
        own.json()["data"].clone()
    };

    let sessions = read("/api/chat/sessions", None);
    assert!(sessions.as_array().unwrap().len() >= 3, "{sessions}");
    // the typed reads (#430), each answering the shape its route says
    for target in [
        "/api/chat/statuses?usernames=wxid_bob&usernames=room1%40chatroom",
        "/api/chat/counts?sessions=wxid_bob&sessions=room1%40chatroom",
        "/api/chat/tab-counts",
        "/api/chat/dates?session-id=wxid_bob",
        "/api/chat/date-counts?session-id=wxid_bob",
        "/api/chat/detail?session-id=wxid_bob&fast=true",
        "/api/chat/detail?session-id=wxid_bob",
        "/api/chat/detail?session-id=wxid_bob&extra=true",
        &format!(
            "/api/chat/image-info?session-id=wxid_bob&msg-id=3&create-time={}",
            T0 + DAY
        ),
        "/api/group/members?chatroom-id=room1%40chatroom&counts=true",
        "/api/group/hours?chatroom-id=room1%40chatroom",
        "/api/group/media?chatroom-id=room1%40chatroom",
        "/api/group/member?chatroom-id=room1%40chatroom&username=wxid_bob",
        "/api/group/member-messages?chatroom-id=room1%40chatroom&username=wxid_bob",
        "/api/analytics/overall",
        "/api/analytics/time",
    ] {
        let data = read(target, None);
        assert!(
            data.as_object().is_some_and(|o| !o.is_empty()),
            "{target}: {data}"
        );
    }
    for target in [
        "/api/group/list",
        "/api/group/ranking?chatroom-id=room1%40chatroom",
        "/api/analytics/rankings",
        "/api/analytics/exclude-candidates",
    ] {
        let data = read(target, None);
        assert!(!data.as_array().unwrap().is_empty(), "{target}: {data}");
    }
    read("/api/analytics/excluded", None);
    for target in [
        "/api/sns/timeline?limit=20",
        "/api/sns/users",
        "/api/sns/stats",
        "/api/sns/post-counts",
        "/api/sns/post-counts?user=wxid_bob",
        "/api/report/annual/years",
        "/api/biz/accounts",
        "/api/biz/messages?username=gh_news",
        "/api/biz/pay-records",
        "/api/config/profiles",
        "/api/runtime/info",
        "/api/cache/list",
        "/api/db/detect",
        "/api/db/wxid",
        "/api/db/test",
        "/api/insight/footprint",
    ] {
        read(target, None);
    }
    for target in [
        "/api/chat/messages?session-id=wxid_bob&limit=100",
        "/api/chat/messages?session-id=room1%40chatroom",
        "/api/chat/latest?session-id=wxid_bob&limit=2",
        "/api/chat/search?keyword=hello",
        "/api/chat/contacts",
    ] {
        let rows = read(target, Some(list_of("Row")));
        assert!(!rows.as_array().unwrap().is_empty(), "{target}");
    }
    for target in [
        "/api/chat/messages?session-id=room1%40chatroom&parsed=true",
        "/api/chat/search?keyword=hello&parsed=true",
    ] {
        let messages = read(target, Some(list_of("ParsedMessage")));
        assert!(!messages.as_array().unwrap().is_empty(), "{target}");
        // a key names the database, not where it is on this machine (#443)
        for m in messages.as_array().unwrap() {
            let key = m["messageKey"].as_str().unwrap();
            assert!(
                key.contains("message_") && !key.contains("%2F") && !key.contains("%5C"),
                "{key}"
            );
        }
    }
    let contacts = read("/api/chat/contacts?parsed=true", Some(list_of("Contact")));
    assert!(contacts.as_array().unwrap().len() >= 5, "{contacts}");
    read(
        "/api/chat/message?session-id=wxid_bob&local-id=2",
        Some(component("MessageLookup")),
    );
    let message = read(
        "/api/chat/message?session-id=wxid_bob&local-id=8&parsed=true",
        Some(component("ParsedMessage")),
    );
    assert_eq!(message["senderUsername"], Value::Null, "{message}");

    // pages of 2, older and older from the cursor of each page's last message, list every message once and in the
    // order of the whole list; so do pages of newer ones from the first message of each
    let parsed = Some(list_of("ParsedMessage"));
    let page = |from: &str| {
        read(
            &format!("/api/chat/messages?session-id=wxid_bob&parsed=true&limit=2{from}"),
            parsed.clone(),
        )
    };
    let cursors = |page: &Value| -> Vec<String> {
        page.as_array()
            .unwrap()
            .iter()
            .map(|m| m["_cursor"].as_str().unwrap().to_string())
            .collect()
    };
    let all = cursors(&read(
        "/api/chat/messages?session-id=wxid_bob&parsed=true&limit=100",
        parsed.clone(),
    ));
    assert_eq!(all.len(), 8, "{all:?}");
    let mut seen = std::collections::HashSet::new();
    assert!(
        all.iter().all(|c| seen.insert(c)),
        "a cursor twice: {all:?}"
    );
    let mut walked = cursors(&page(""));
    loop {
        let next = cursors(&page(&format!(
            "&before={}",
            percent(walked.last().unwrap())
        )));
        if next.is_empty() {
            break;
        }
        walked.extend(next);
    }
    assert_eq!(walked, all);
    let mut forward = vec![all.last().unwrap().clone()];
    loop {
        let next = cursors(&page(&format!("&after={}", percent(&forward[0]))));
        if next.is_empty() {
            break;
        }
        forward.splice(0..0, next);
    }
    assert_eq!(forward, all);

    // the resources of every conversation, a page of one at a time, until a page says no more follow
    let resources = Some(component("ResourcePage"));
    let whole = read("/api/chat/resources?limit=100", resources.clone());
    let keys = |page: &Value| -> Vec<String> {
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["_cursor"].as_str().unwrap().to_string())
            .collect()
    };
    assert!(keys(&whole).len() >= 2, "{whole}");
    assert_eq!(whole["nextCursor"], Value::Null);
    let mut listed = Vec::new();
    let mut before = String::new();
    loop {
        let page = read(
            &format!("/api/chat/resources?limit=1{before}"),
            resources.clone(),
        );
        listed.extend(keys(&page));
        if page["hasMore"] == false {
            assert_eq!(page["nextCursor"], Value::Null);
            break;
        }
        assert_eq!(
            page["nextCursor"].as_str(),
            keys(&page).last().map(String::as_str)
        );
        before = format!("&before={}", percent(page["nextCursor"].as_str().unwrap()));
    }
    assert_eq!(listed, keys(&whole));

    // failures: a missing argument, a cursor of nothing, an image not on this computer (whose route without `/api/`
    // writes a file, by POST)
    let failure = answer_schema("/api/chat/messages", "default");
    let image = format!(
        "/api/chat/image-data?session-id=wxid_bob&msg-id=3&create-time={}",
        T0 + DAY
    );
    for (target, status) in [
        ("/api/chat/messages", 400),
        ("/api/chat/messages?session-id=wxid_bob&before=1.2.3", 400),
        ("/chat/messages", 400),
        ("/chat/messages?session-id=wxid_bob&before=1.2.3", 400),
        (image.as_str(), 404),
    ] {
        let r = server.call("GET", target, "");
        assert_eq!(r.status, status, "{target}: {}", r.body);
        check(target, &failure, &r.json());
    }
    let r = server.call("GET", &image, "");
    assert_eq!(r.json()["error"]["details"]["failureKind"], "not_found");

    // a job, while it runs and once it has ended, and the list of jobs
    let auth = format!("Bearer {TOKEN}");
    let r = server.request(
        "GET",
        "/chat/sessions",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    assert_eq!(r.status, 202, "{}", r.body);
    let job_answer = answer_schema("/jobs/{id}", "200");
    check("the job started", &job_answer, &r.json());
    let id = r.json()["data"]["id"].as_str().unwrap().to_string();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let r = server.call("GET", &format!("/jobs/{id}"), "");
        check("the job", &job_answer, &r.json());
        if r.json()["data"]["status"] == "succeeded" {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{}", r.body);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let jobs = server.call("GET", "/jobs", "").json();
    check("the jobs", &list_of("Job"), &jobs["data"]);
}
