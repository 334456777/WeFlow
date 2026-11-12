//! The server itself: the token, `/health`, its language, answering only this machine, the WebUI it serves and the
//! message it starts with.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{json, Value};
use weflow_core::contract::API_VERSION;

use super::common::{account_home, run_cli, start, start_env, temp_root, weflow, TOKEN};

#[test]
fn options_saved_in_the_config_apply_where_the_command_line_gives_none() {
    let root = temp_root("saved");
    for (key, value) in [
        ("serve_max_tasks", "3"),
        ("serve_max_reads", "9"),
        ("serve_log_requests", "true"),
    ] {
        let out = weflow(&root)
            .args(["config", "set", key, value])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{key}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let out = weflow(&root)
        .args(["config", "set", "serve_port", "99999"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let server = start(&root, &["--api-token", TOKEN, "--max-reads", "5"]);
    let limits = server.request("GET", "/health", &[], "").json()["limits"].clone();
    assert_eq!(limits["maxTasks"], 3);
    assert_eq!(limits["maxReads"], 5);
}

#[test]
fn health_needs_no_token_and_everything_else_does() {
    let root = temp_root("auth");
    let server = start(&root, &["--api-token", TOKEN]);
    let r = server.request("GET", "/health", &[], "");
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["status"], "ok");
    assert_eq!(r.json()["apiVersion"], API_VERSION);
    // how it runs, for the settings (#214)
    assert_eq!(r.json()["limits"]["maxTasks"], 1);
    assert!(r.json()["limits"]["maxReads"].as_u64().unwrap() >= 1);
    assert_eq!(r.json()["insight"], false);
    drop(server);
    let server = start(
        &root,
        &[
            "--api-token",
            TOKEN,
            "--max-tasks",
            "2",
            "--max-reads",
            "3",
            "--max-media",
            "4",
        ],
    );
    let limits = server.request("GET", "/health", &[], "").json()["limits"].clone();
    assert_eq!(
        limits,
        json!({ "maxTasks": 2, "maxReads": 3, "maxMedia": 4 })
    );

    let r = server.request("GET", "/config/path", &[], "");
    assert_eq!(r.status, 401);
    assert_eq!(r.json()["error"]["code"], "unauthorized");
    let r = server.request(
        "GET",
        "/config/path",
        &[("Authorization", "Bearer nope")],
        "",
    );
    assert_eq!(r.status, 401);
    let r = server.request(
        "GET",
        &format!("/config/path?access_token={TOKEN}"),
        &[],
        "",
    );
    assert_eq!(r.status, 200);
}

#[test]
fn health_data_changes_with_the_configuration_and_each_run() {
    let root = temp_root("data");
    let server = start(&root, &["--api-token", TOKEN]);
    let data = || {
        server.request("GET", "/health", &[], "").json()["data"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let first = data();
    assert_eq!(first.len(), 16);
    assert_eq!(data(), first);
    // another account set up while the server runs
    let (code, _, err) = run_cli(&root, &["config", "set", "myWxid", "wxid_synthetic_b"]);
    assert_eq!(code, 0, "{err}");
    let second = data();
    assert_ne!(second, first);
    assert_eq!(data(), second);
    drop(server);
    // the same configuration, another run
    let server = start(&root, &["--api-token", TOKEN]);
    let third = server.request("GET", "/health", &[], "").json()["data"].clone();
    assert_ne!(third, json!(second));
}

#[test]
fn the_api_describes_itself() {
    let root = temp_root("openapi");
    let server = start(&root, &["--api-token", TOKEN]);
    assert_eq!(server.request("GET", "/openapi.json", &[], "").status, 401);
    let auth = format!("Bearer {TOKEN}");
    let get = |lang: &str| {
        let r = server.request(
            "GET",
            "/openapi.json",
            &[("Authorization", &auth), ("Accept-Language", lang)],
            "",
        );
        assert_eq!(r.status, 200);
        r.json()
    };
    let en = get("en");
    assert_eq!(en["openapi"], "3.1.0");
    let health = server.request("GET", "/health", &[], "").json();
    assert_eq!(en["info"]["x-weflow-api-version"], health["apiVersion"]);
    assert_eq!(en["info"]["version"], health["version"]);
    for path in ["/chat/sessions", "/backup/create", "/jobs/{id}/events"] {
        assert!(en["paths"][path].is_object(), "{path} is missing");
    }
    assert!(en["paths"]["/serve"].is_null());
    // in the server's language, whatever the request asks for (#433)
    assert_eq!(get("zh-CN"), en);
    assert_eq!(server.call("POST", "/openapi.json", "").status, 405);
    let zh_root = temp_root("openapi-zh");
    let zh_server = start_env(&zh_root, &["--api-token", TOKEN], &[("WEFLOW_LANG", "zh")]);
    let zh = zh_server
        .request(
            "GET",
            "/openapi.json",
            &[("Authorization", &auth), ("Accept-Language", "en")],
            "",
        )
        .json();
    let summary = |d: &Value| d["paths"]["/chat/sessions"]["get"]["summary"].clone();
    assert_ne!(summary(&en), summary(&zh));
}

/// Every answer is in the server's language, from its process or a child's, whatever the request's
/// `Accept-Language` (#433).
#[test]
fn answers_are_in_the_server_s_language() {
    let root = temp_root("language");
    let home = account_home(&root);
    let server = start_env(&home, &["--api-token", TOKEN], &[("WEFLOW_LANG", "zh")]);
    let auth = format!("Bearer {TOKEN}");
    let chinese = |text: &str| text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    let mut seen = Vec::new();
    // a usage error, and an error of the read itself
    for target in ["/chat/messages", "/chat/message?session-id=wxid_bob"] {
        for route in [target.to_string(), format!("/api{target}")] {
            for lang in ["en", "zh-CN"] {
                let r = server.request(
                    "GET",
                    &route,
                    &[("Authorization", &auth), ("Accept-Language", lang)],
                    "",
                );
                assert_eq!(r.status, 400, "{route}: {}", r.body);
                let message = r.json()["error"]["message"].as_str().unwrap().to_string();
                assert!(chinese(&message), "{route} {lang}: {message}");
                seen.push((target, message));
            }
        }
    }
    // one message for each, with or without `/api/`
    seen.dedup();
    assert_eq!(seen.len(), 2, "{seen:#?}");
    // and the server says which, for the WebUI to show itself in (#439)
    let health = server.request("GET", "/health", &[("Accept-Language", "en")], "");
    assert_eq!(health.json()["lang"], "zh");
    drop(server);
    let english = start_env(&home, &["--api-token", TOKEN], &[("WEFLOW_LANG", "en")]);
    let health = english.request("GET", "/health", &[("Accept-Language", "zh-CN")], "");
    assert_eq!(health.json()["lang"], "en");
}

#[test]
fn only_requests_to_this_machine_are_answered() {
    let root = temp_root("host");
    let server = start(&root, &["--api-token", TOKEN]);
    let port = server.addr.rsplit_once(':').unwrap().1.to_string();
    let auth = format!("Bearer {TOKEN}");
    for host in ["evil.example", "evil.example:80", "localhost.evil.example"] {
        let host = host.replace(":80", &format!(":{port}"));
        for target in ["/health", "/config/path"] {
            let r = server.request(
                "GET",
                target,
                &[("Host", &host), ("Authorization", &auth)],
                "",
            );
            assert_eq!(r.status, 403, "{host} {target}");
            assert_eq!(r.json()["error"]["code"], "invalid_host");
        }
    }
    for host in [format!("localhost:{port}"), format!("127.0.0.1:{port}")] {
        let r = server.request(
            "GET",
            "/config/path",
            &[("Host", &host), ("Authorization", &auth)],
            "",
        );
        assert_eq!(r.status, 200, "{host}");
    }
}

/// A built WebUI's folder: its page, an asset, and a secret beside the folder that must stay out of reach.
fn webui(root: &Path) -> PathBuf {
    let dist = root.join("dist");
    std::fs::create_dir_all(dist.join("assets")).unwrap();
    std::fs::write(
        dist.join("index.html"),
        "<!doctype html><title>WeFlow</title>",
    )
    .unwrap();
    std::fs::write(dist.join("assets/index-a1b2.js"), "console.log(1)").unwrap();
    std::fs::write(dist.join("weflow-web.json"), "{}").unwrap();
    std::fs::write(root.join("secret.txt"), "not for the page").unwrap();
    dist
}

#[test]
fn the_webui_is_served_under_ui() {
    let root = temp_root("webui");
    let dist = webui(&root);
    let server = start(
        &root,
        &["--api-token", TOKEN, "--ui-dir", dist.to_str().unwrap()],
    );
    let get = |target: &str| server.request("GET", target, &[], "");

    // the page and its files need no token
    let page = get("/ui/");
    assert_eq!(page.status, 200);
    assert!(page.body.contains("<title>WeFlow"));
    assert_eq!(page.headers["content-type"], "text/html; charset=utf-8");
    assert_eq!(page.headers["cache-control"], "no-cache");
    assert_eq!(page.headers["referrer-policy"], "no-referrer");
    assert_eq!(page.headers["x-content-type-options"], "nosniff");
    let script = get("/ui/assets/index-a1b2.js");
    assert_eq!(script.status, 200);
    assert_eq!(script.body, "console.log(1)");
    assert_eq!(
        script.headers["content-type"],
        "text/javascript; charset=utf-8"
    );
    assert!(script.headers["cache-control"].contains("immutable"));
    assert_eq!(
        get("/ui/weflow-web.json").headers["cache-control"],
        "no-cache"
    );

    // the page's own paths get the page; missing files and API paths do not
    assert!(get("/ui/chat/wxid_a").body.contains("<title>WeFlow"));
    assert_eq!(get("/ui/assets/missing.js").status, 404);
    assert_eq!(get("/ui/assets/chunk").status, 404);
    assert_eq!(get("/ui/missing.png").status, 404);
    let api = server.call("GET", "/chat/no-such-command", "");
    assert_eq!(api.status, 404);
    assert_eq!(api.json()["success"], false);

    // nothing outside the folder
    for target in [
        "/ui/../secret.txt",
        "/ui/%2e%2e/secret.txt",
        "/ui/assets/..%2f..%2fsecret.txt",
        "/ui/..%5csecret.txt",
    ] {
        let r = get(target);
        assert!(!r.body.contains("not for the page"), "{target}");
        assert_ne!(r.status, 200, "{target}");
    }

    // into the page, keeping the token in the address
    let r = get("/ui?token=abc");
    assert_eq!(r.status, 308);
    assert_eq!(r.headers["location"], "/ui/?token=abc");
    let r = get("/?token=abc");
    assert_eq!(r.status, 302);
    assert_eq!(r.headers["location"], "/ui/?token=abc");
    // read-only, and still only for this machine
    assert_eq!(server.call("POST", "/ui/", "").status, 405);
    let r = server.request("GET", "/ui/", &[("Host", "evil.example")], "");
    assert_eq!(r.status, 403);
}

#[test]
fn a_link_out_of_the_webui_folder_is_not_followed() {
    let root = temp_root("webui-link");
    let dist = webui(&root);
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(root.join("secret.txt"), dist.join("leak.txt"));
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(root.join("secret.txt"), dist.join("leak.txt"));
    if linked.is_err() {
        // Windows without the right to make links
        return;
    }
    let server = start(
        &root,
        &["--api-token", TOKEN, "--ui-dir", dist.to_str().unwrap()],
    );
    let r = server.request("GET", "/ui/leak.txt", &[], "");
    assert_eq!(r.status, 404);
    assert!(!r.body.contains("not for the page"));
}

#[test]
fn without_a_webui_the_api_runs_alone() {
    let root = temp_root("no-webui");
    let mut child = weflow(&root)
        .args(["--json", "serve", "--port", "0", "--api-token", TOKEN])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let event: Value = loop {
        let line = lines.next().unwrap().unwrap();
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            break v;
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(event["ui"], false);
    assert!(event["uiUrl"].is_null());

    let server = start(&root, &["--api-token", TOKEN]);
    let r = server.request("GET", "/ui/", &[], "");
    assert_eq!(r.status, 404);
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--ui-dir"));
    // `/` is not taken over
    assert_eq!(server.request("GET", "/", &[], "").status, 401);

    // a folder without a page is refused at the start
    let (code, _, err) = run_cli(
        &root,
        &["serve", "--port", "0", "--ui-dir", root.to_str().unwrap()],
    );
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("index.html"), "{err}");
}

#[test]
fn the_startup_message_gives_the_webui_address_with_the_token() {
    let root = temp_root("webui-url");
    let dist = webui(&root);
    let mut child = weflow(&root)
        .args([
            "--json",
            "serve",
            "--port",
            "0",
            "--api-token",
            TOKEN,
            "--ui-dir",
        ])
        .arg(&dist)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let event: Value = loop {
        let line = lines.next().unwrap().unwrap();
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            break v;
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(event["ui"], true);
    let url = event["uiUrl"].as_str().unwrap();
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    assert!(url.ends_with(&format!("/ui/?token={TOKEN}")), "{url}");
}

#[test]
fn a_token_is_made_up_when_none_is_configured() {
    let root = temp_root("gen-token");
    let mut child = weflow(&root)
        .args(["--json", "serve", "--port", "0"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let event: Value = loop {
        let line = lines.next().unwrap().unwrap();
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            break v;
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(event["tokenConfigured"], false);
    assert_eq!(event["generatedToken"].as_str().unwrap().len(), 32);
}

#[test]
fn a_missing_account_is_a_conflict() {
    let root = temp_root("no-account");
    let server = start(&root, &["--api-token", TOKEN]);
    let r = server.call("GET", "/chat/sessions", "");
    assert_eq!(r.status, 409, "{}", r.body);
    assert_eq!(r.json()["error"]["code"], "config_error");
}
