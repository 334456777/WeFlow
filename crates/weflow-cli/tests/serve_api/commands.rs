//! Commands run in a child process: answered as the terminal answers, the account commands, the refused ones and
//! the keys the server hands over.

use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;

use serde_json::{json, Value};

use super::common::{account_home, account_home_with, run_cli, start, temp_root, weflow, TOKEN};

#[test]
fn commands_answer_like_the_terminal() {
    let root = temp_root("config");
    let server = start(&root, &["--api-token", TOKEN]);
    let home = &server.home;

    // the default answer is the `--json` document itself
    let r = server.call("GET", "/config/path", "");
    let (exit, stdout, _) = run_cli(home, &["--json", "config", "path"]);
    assert_eq!((r.status, exit), (200, 0));
    assert_eq!(r.body, stdout);
    assert_eq!(r.headers["x-weflow-exit-code"], "0");

    // a setting goes in with PUT and comes back with GET
    let r = server.call(
        "PUT",
        "/config/set",
        r#"{"key": "no_progress", "value": "true"}"#,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let r = server.call("GET", "/config/get?key=no_progress", "");
    assert_eq!(r.status, 200);
    let (_, stdout, _) = run_cli(home, &["--json", "config", "get", "no_progress"]);
    assert_eq!(r.body, stdout);

    // `std=in` shows the command without running it
    let r = server.call("GET", "/config/get?key=no_progress&std=in", "");
    assert_eq!(
        r.json(),
        json!({ "argv": ["weflow", "--json", "config", "get", "--", "no_progress"], "stdin": null })
    );

    // `std=out` / `std=err` carry the human-readable run
    let r = server.call("GET", "/config/get?key=no_progress&std=out", "");
    let (_, stdout, stderr) = run_cli(home, &["config", "get", "no_progress"]);
    assert_eq!(r.json(), json!({ "exitCode": 0, "stdout": stdout }));
    let r = server.call("GET", "/config/get?key=no_progress&std=err", "");
    assert_eq!(r.json(), json!({ "exitCode": 0, "stderr": stderr }));
    let r = server.call("GET", "/config/path?std=all", "");
    assert_eq!(r.status, 400);

    // the command checks its arguments as on a terminal: exit code 2 is 400
    let r = server.call("GET", "/config/get?key=a&key=b", "");
    let (exit, stdout, _) = run_cli(home, &["--json", "config", "get", "--", "a", "b"]);
    assert_eq!((r.status, exit), (400, 2));
    assert_eq!(r.body, stdout);
    assert_eq!(r.headers["x-weflow-error-code"], "invalid_arguments");
    // the server refuses names the command does not have
    let r = server.call("GET", "/config/path?bogus=1", "");
    assert_eq!(r.status, 400);
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("bogus"));

    // the method follows the command
    let r = server.call("POST", "/config/path", "");
    assert_eq!((r.status, r.headers["allow"].as_str()), (405, "GET"));
    let r = server.call("PUT", "/analytics/excluded", r#"{"set": "x"}"#);
    assert_ne!(r.status, 405);
    let r = server.call("PUT", "/analytics/excluded", "");
    assert_eq!((r.status, r.headers["allow"].as_str()), (405, "GET"));

    // unknown paths and `serve` itself are not commands
    for path in ["/nope", "/serve", "/config", "/"] {
        assert_eq!(server.call("GET", path, "").status, 404, "{path}");
    }
    let r = server.call("PUT", "/config/set", "[1]");
    assert_eq!(r.status, 400);
}

#[test]
fn account_commands_and_read_only_refusals() {
    let root = temp_root("account");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);

    let r = server.call("GET", "/chat/sessions", "");
    let (_, stdout, _) = run_cli(&home, &["--json", "chat", "sessions"]);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.body, stdout);
    assert!(!r.json()["data"].as_array().unwrap().is_empty());

    // writing WeChat's databases is refused: 501 with the `unsupported` code
    let r = server.call(
        "DELETE",
        "/chat/delete-message",
        r#"{"session-id": "wxid_friend", "local-id": 1, "create-time": 1}"#,
    );
    assert_eq!(r.status, 501, "{}", r.body);
    assert_eq!(r.json()["error"]["code"], "unsupported");
    assert_eq!(r.headers["x-weflow-exit-code"], "4");
    let r = server.call("DELETE", "/sns/block-delete?action=uninstall", "");
    assert_eq!(r.status, 501, "{}", r.body);
    let r = server.call("GET", "/sns/block-delete?action=uninstall", "");
    assert_eq!((r.status, r.headers["allow"].as_str()), (405, "DELETE"));
}

#[test]
fn clearing_an_account_can_be_looked_at_first() {
    let root = temp_root("clear-dry");
    let home = account_home(&root);
    let kept = home.join("cache").join("Images").join("wxid_me_ab12");
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::write(kept.join("a.jpg"), b"jpeg").unwrap();
    let server = start(&home, &["--api-token", TOKEN]);

    // what would go, and nothing goes
    let r = server.call("DELETE", "/cache/clear-account?dry-run=true", "");
    assert_eq!(r.status, 200, "{}", r.body);
    let plan = &r.json()["data"];
    assert_eq!(plan["dryRun"], true);
    let paths: Vec<PathBuf> = plan["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| PathBuf::from(p["path"].as_str().unwrap()))
        .collect();
    assert!(
        paths.iter().any(|p| p.ends_with("wxid_me_ab12")),
        "{paths:?}"
    );
    assert!(plan["bytes"].as_u64().unwrap() >= 4);
    assert_eq!(plan["profileKeys"][1], "wxid");
    assert!(kept.join("a.jpg").exists());
    let (_, wxid, _) = run_cli(&home, &["config", "get", "wxid"]);
    assert_eq!(wxid.trim(), "wxid_me_ab12");

    // and only with `yes`, without `dry-run`
    let r = server.call("DELETE", "/cache/clear-account?dry-run=true&yes=true", "");
    assert_eq!(r.status, 400, "{}", r.body);
}

#[test]
fn refused_commands_are_marked_and_refused() {
    let root = temp_root("refused");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let doc = server.call("GET", "/openapi.json", "").json();
    let mut marked = Vec::new();
    for (path, ops) in doc["paths"].as_object().unwrap() {
        for (method, op) in ops.as_object().unwrap() {
            if op.get("x-weflow-refused") != Some(&Value::Bool(true)) {
                continue;
            }
            marked.push(format!("{} {path}", method.to_uppercase()));
            // its required parameters and its lists (the conversations of `anti-revoke install`, which refuses
            // each one), made up, in the query: they are taken from there for every method
            let mut query = Vec::new();
            let mut take = |name: &str, schema: &Value| {
                let value = match (schema.get("enum"), schema["type"].as_str()) {
                    (Some(values), _) => values[0].as_str().unwrap().to_string(),
                    (_, Some("integer")) => "1".to_string(),
                    _ => "x".to_string(),
                };
                query.push(format!("{name}={value}"));
            };
            for p in op["parameters"].as_array().unwrap() {
                if p["required"] == true || p["schema"]["type"] == "array" {
                    take(p["name"].as_str().unwrap(), &p["schema"]);
                }
            }
            let body = &op["requestBody"]["content"]["application/json"]["schema"];
            let required = body["required"].as_array().cloned().unwrap_or_default();
            for (name, schema) in body["properties"].as_object().into_iter().flatten() {
                if required.contains(&Value::from(name.as_str())) || schema["type"] == "array" {
                    take(name, schema);
                }
            }
            let r = server.call(
                &method.to_uppercase(),
                &format!("{path}?{}", query.join("&")),
                "",
            );
            assert_eq!(r.status, 501, "{method} {path}: {}", r.body);
            assert_eq!(r.json()["error"]["code"], "unsupported");
        }
    }
    marked.sort();
    assert_eq!(
        marked,
        [
            "DELETE /chat/anti-revoke/uninstall",
            "DELETE /chat/delete-message",
            "DELETE /sns/block-delete",
            "DELETE /sns/delete",
            "GET /chat/anti-revoke/check",
            "GET /sns/block-delete",
            "PATCH /chat/update-message",
            "POST /chat/mark-read",
            "PUT /chat/anti-revoke/install",
            "PUT /sns/block-delete",
        ]
    );
}

#[test]
fn commands_answer_with_the_keys_the_server_hands_over() {
    let root = temp_root("keys");
    let home = account_home(&root);
    let (exit, terminal, stderr) = run_cli(&home, &["--json", "chat", "sessions"]);
    assert_eq!(exit, 0, "{stderr}");

    // once the server has opened the databases of a chat page, the commands it runs get their keys
    let server = start(&home, &["--api-token", TOKEN]);
    assert_eq!(server.call("GET", "/api/chat/sessions", "").status, 200);
    let r = server.call("GET", "/chat/sessions", "");
    assert_eq!((r.status, &r.body), (200, &terminal));

    // keys that do not open the files, a cut-off record or none at all: the keys are derived again
    for given in [vec![0u8; 80 * 3], vec![1u8; 7], Vec::new()] {
        let mut child = weflow(&home)
            .args(["--json", "chat", "sessions"])
            .env("WEFLOW_KEYS_ON_STDIN", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&given).unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), terminal);
    }
}

/// `key db` run as the server runs it ends by itself once its stdin ends, with `user_interrupt`, as on Ctrl+C: a
/// hook it had set would be out of WeChat first (#287). It is ended here while it waits for WeChat, before any hook.
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn key_db_ends_by_itself_when_its_stdin_ends() {
    use std::io::{BufRead, BufReader, Read};

    let root = temp_root("key-db-stdin");
    let mut child = weflow(&root)
        .env("WEFLOW_CANCEL_ON_STDIN", "1")
        .args(["--json", "key", "db", "--timeout", "60"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // waiting for WeChat to quit or to start, whichever this machine needs
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let phase = loop {
        let line = lines
            .next()
            .expect("key db ended before it waited")
            .unwrap();
        if let Ok(event) = serde_json::from_str::<Value>(&line) {
            if event["type"] == "key_phase" {
                break event["phase"].clone();
            }
        }
    };
    assert!(phase == "quit_wechat" || phase == "open_wechat", "{phase}");
    let ended = std::time::Instant::now();
    drop(child.stdin.take());
    std::thread::spawn(move || lines.for_each(drop));
    let status = child.wait().unwrap();
    assert!(ended.elapsed() < std::time::Duration::from_secs(5));
    let mut out = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut out)
        .unwrap();
    let doc: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["error"]["code"], "user_interrupt", "{out}");
    assert_eq!(status.code(), Some(130));
}

/// A text message loses its first line only when that line names its sender, as WeChat writes the messages a group
/// receives (#656): a private message or a sent one that starts like `PS:` keeps it, in the terminal, in the
/// server's reads and in an export.
#[test]
fn only_a_first_line_naming_the_sender_is_taken_off_a_text() {
    use weflow_native::fixture::{MsgSpec, DAY, T0};
    let root = temp_root("sender-line");
    let at = T0 + 5 * DAY;
    let home = account_home_with(&root, |f| {
        f.message_shard(
            2,
            &[
                (
                    "wxid_carol",
                    vec![
                        MsgSpec::text(1, "wxid_carol", at, "PS:\nthe file is attached"),
                        MsgSpec::text(2, "wxid_me", at + 60, "Note:\nlater"),
                    ],
                ),
                (
                    "room1@chatroom",
                    vec![
                        MsgSpec::text(10, "wxid_bob", at + 120, "wxid_bob:\nTODO:\nbuy milk"),
                        MsgSpec::text(11, "wxid_me", at + 180, "Plan:\nmeet at 5"),
                    ],
                ),
            ],
        );
    });
    let expected = [
        ("wxid_carol", 1, "PS:\nthe file is attached"),
        ("wxid_carol", 2, "Note:\nlater"),
        ("room1@chatroom", 10, "TODO:\nbuy milk"),
        ("room1@chatroom", 11, "Plan:\nmeet at 5"),
    ];
    let text_of = |messages: &Value, local_id: i64| {
        messages["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["localId"] == local_id)
            .map(|m| m["parsedContent"].as_str().unwrap().to_string())
            .unwrap_or_else(|| panic!("message {local_id} in {messages}"))
    };

    let server = start(&home, &["--api-token", TOKEN]);
    for session in ["wxid_carol", "room1@chatroom"] {
        let (exit, stdout, stderr) =
            run_cli(&home, &["--json", "chat", "messages", session, "--parsed"]);
        assert_eq!(exit, 0, "{stdout}{stderr}");
        let terminal: Value = serde_json::from_str(&stdout).unwrap();
        let target = format!(
            "/api/chat/messages?session-id={}&parsed=true",
            session.replace('@', "%40")
        );
        let own = server.call("GET", &target, "");
        assert_eq!(own.status, 200, "{target}: {}", own.body);
        for &(_, local_id, text) in expected.iter().filter(|(s, ..)| *s == session) {
            assert_eq!(text_of(&terminal, local_id), text, "{session} {local_id}");
            assert_eq!(text_of(&own.json(), local_id), text, "{target} {local_id}");
        }
    }

    // the plain-text export of each conversation writes the same text
    for session in ["wxid_carol", "room1@chatroom"] {
        let out = root.join(format!("{}.txt", session.replace('@', "_")));
        let (exit, stdout, stderr) = run_cli(
            &home,
            &[
                "--json",
                "export",
                "messages",
                session,
                "--format",
                "txt",
                "--out",
                out.to_str().unwrap(),
            ],
        );
        assert_eq!(exit, 0, "{stdout}{stderr}");
        let txt = std::fs::read_to_string(&out).unwrap();
        for &(_, local_id, text) in expected.iter().filter(|(s, ..)| *s == session) {
            assert!(txt.contains(text), "{session} {local_id}:\n{txt}");
        }
        assert!(!txt.contains("wxid_bob:\nTODO"), "{txt}");
    }
}
