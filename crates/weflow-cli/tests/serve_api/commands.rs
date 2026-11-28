//! Commands run in a child process: answered as the terminal answers, the account commands, the refused ones and
//! the keys the server hands over.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::{json, Value};

use super::common::{account_home, account_home_with, run_cli, start, temp_root, weflow, TOKEN};

/// An answer that nothing reads any more is dropped, and the command exits with its own code (#989).
#[test]
fn answers_nobody_reads_are_dropped() {
    let root = temp_root("no-reader");
    for args in [
        &["config", "path"][..],
        &["--json", "config", "path"],
        &["--help"],
    ] {
        let mut child = weflow(&root)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // the reader is gone before the command gets to its answer
        drop(child.stdout.take());
        let out = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {stderr}");
        assert!(!stderr.contains("panicked"), "{args:?}: {stderr}");
    }
}

/// Help and parse errors are in the language of the run, the help texts of the command tree included, and a command
/// line that parses runs in either language: the tree is translated only for help and errors (`i18n::parse`).
#[test]
fn help_and_parse_errors_follow_the_language() {
    let root = temp_root("help-language");
    let run = |lang: &str, args: &[&str]| {
        let out = weflow(&root)
            .env("WEFLOW_LANG", lang)
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    for (lang, path_help, set_about, error) in [
        (
            "en",
            "Show the path of the config file",
            "Set a key",
            "error: unexpected argument '--bogus' found",
        ),
        (
            "zh",
            "显示配置文件路径",
            "设置一个键",
            "错误：发现意外的参数 '--bogus'",
        ),
    ] {
        let (code, stdout, _) = run(lang, &["config", "--help"]);
        assert_eq!(code, Some(0), "{lang}");
        assert!(stdout.contains(path_help), "{lang}: {stdout}");
        // a command given none of its required arguments shows its help as an error
        let (code, _, stderr) = run(lang, &["config", "set"]);
        assert_eq!(code, Some(2), "{lang}");
        assert!(stderr.contains(set_about), "{lang}: {stderr}");
        let (code, _, stderr) = run(lang, &["config", "path", "--bogus"]);
        assert_eq!(code, Some(2), "{lang}");
        assert!(stderr.contains(error), "{lang}: {stderr}");
        let (code, stdout, stderr) = run(lang, &["config", "path"]);
        assert_eq!(code, Some(0), "{lang}: {stderr}");
        assert!(stdout.contains("config.json"), "{lang}: {stdout}");
    }
}

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
    let r = server.call("GET", "/chat/messages?session-id=x&limit=many", "");
    let (exit, stdout, _) = run_cli(
        home,
        &["--json", "chat", "messages", "--limit=many", "--", "x"],
    );
    assert_eq!((r.status, exit), (400, 2));
    assert_eq!(r.body, stdout);
    assert_eq!(r.headers["x-weflow-error-code"], "invalid_arguments");
    // a positional argument given twice is refused by the server: on the command line the second value would go
    // to the next argument
    let r = server.call("GET", "/config/get?key=a&key=b", "");
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["error"]["code"], "invalid_arguments");
    assert!(r.body.contains("`key`"), "{}", r.body);
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

/// A report year that is neither 0 nor 1970-2100 is refused on the terminal and by the server alike, not read as all
/// the years.
#[test]
fn a_report_year_outside_1970_to_2100_is_refused() {
    let root = temp_root("report-year");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    for (route, body, args) in [
        (
            "/report/annual/generate",
            r#"{"year": 1969}"#,
            &["annual", "generate", "--year=1969"][..],
        ),
        (
            "/report/annual/generate",
            r#"{"year": -5}"#,
            &["annual", "generate", "--year=-5"],
        ),
        (
            "/report/dual/generate",
            r#"{"friend": "wxid_bob", "year": 2101}"#,
            &["dual", "generate", "--year=2101", "--friend=wxid_bob"],
        ),
    ] {
        let r = server.call("POST", route, body);
        let mut argv = vec!["--json", "report"];
        argv.extend_from_slice(args);
        let (exit, stdout, _) = run_cli(&home, &argv);
        assert_eq!((r.status, exit), (400, 2), "{route} {body}: {}", r.body);
        assert_eq!(r.json()["error"]["code"], "invalid_arguments", "{body}");
        assert_eq!(r.body, stdout, "{route} {body}");
    }
}

/// The names of the profiles `config profiles` lists.
fn profile_names(home: &Path) -> Vec<String> {
    let (exit, stdout, stderr) = run_cli(home, &["--json", "config", "profiles"]);
    assert_eq!(exit, 0, "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    doc["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect()
}

/// `config set` refuses a profile that could not be addressed (#1043), and `config remove-profile` takes a profile
/// out of the config: not the current one, and where there is no terminal to ask on only with `--yes`.
#[test]
fn a_profile_is_made_with_a_name_that_works_and_removed_with_yes() {
    let root = temp_root("remove-profile");
    let set = |key: &str, value: &str| run_cli(&root, &["--json", "config", "set", key, value]);
    for name in ["work", "spare"] {
        let (exit, _, stderr) = set("current_profile", name);
        assert_eq!(exit, 0, "{stderr}");
        let (exit, _, stderr) = set("wxid", &format!("wxid_{name}"));
        assert_eq!(exit, 0, "{stderr}");
    }
    let (exit, _, stderr) = set("current_profile", "home");
    assert_eq!(exit, 0, "{stderr}");
    let all = ["default", "home", "spare", "work"];
    assert_eq!(profile_names(&root), all);

    // a name that no `<profile>.<key>` reaches is refused, and makes no profile
    for bad in ["", "a.b", "wxid", "serve"] {
        let (exit, stdout, _) = set("current_profile", bad);
        let doc: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(
            (exit, &doc["error"]["code"]),
            (2, &json!("invalid_arguments")),
            "{bad:?}: {stdout}"
        );
    }
    assert_eq!(profile_names(&root), all);
    let (_, stdout, _) = run_cli(&root, &["--json", "config", "get", "current_profile"]);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout).unwrap()["data"],
        "home"
    );

    // there is no terminal to ask on here
    let remove = |args: &[&str]| {
        let mut argv = vec!["--json", "config", "remove-profile"];
        argv.extend_from_slice(args);
        let (exit, stdout, _) = run_cli(&root, &argv);
        (exit, serde_json::from_str::<Value>(&stdout).unwrap())
    };
    let (exit, doc) = remove(&["work"]);
    assert_eq!(exit, 2, "{doc}");
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .contains("add --yes"),
        "{doc}"
    );
    // not the current one, not one there is not, and nothing is asked about either
    let (exit, doc) = remove(&["--yes", "home"]);
    assert_eq!(exit, 2, "{doc}");
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .contains("current profile"),
        "{doc}"
    );
    let (exit, doc) = remove(&["nobody"]);
    assert_eq!(exit, 2, "{doc}");
    assert!(
        doc["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no profile named 'nobody'"),
        "{doc}"
    );
    assert_eq!(profile_names(&root), all);

    let (exit, doc) = remove(&["work", "-y"]);
    assert_eq!(exit, 0, "{doc}");
    assert!(doc["data"]["configPath"].is_string(), "{doc}");
    assert_eq!(profile_names(&root), ["default", "home", "spare"]);
    let (_, stdout, _) = run_cli(&root, &["--json", "config", "get", "spare.wxid"]);
    assert_eq!(
        serde_json::from_str::<Value>(&stdout).unwrap()["data"],
        "wxid_spare"
    );

    // through `serve`: a `DELETE`, with `yes` as the terminal has `--yes`
    let server = start(&root, &["--api-token", TOKEN]);
    let r = server.call("GET", "/config/remove-profile?name=spare", "");
    assert_eq!((r.status, r.headers["allow"].as_str()), (405, "DELETE"));
    let r = server.call("DELETE", "/config/remove-profile?name=spare", "");
    assert_eq!(r.status, 400, "{}", r.body);
    assert_eq!(profile_names(&root), ["default", "home", "spare"]);
    let r = server.call("DELETE", "/config/remove-profile?name=spare&yes=true", "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(profile_names(&root), ["default", "home"]);
    let r = server.call("DELETE", "/config/remove-profile?name=home&yes=true", "");
    assert_eq!(r.status, 400, "{}", r.body);
}

/// A switch takes `1` and `yes` as well as `true`, and a value that is none says which key it was for; the progress
/// settings are checked as the others are, and `no_progress` is read whichever way a file wrote it (#1042).
#[test]
fn switches_take_every_spelling_and_no_progress_is_read_as_one() {
    let root = temp_root("config-switches");
    let get = |key: &str| {
        let (exit, stdout, stderr) = run_cli(&root, &["--json", "config", "get", key]);
        assert_eq!(exit, 0, "{stderr}");
        serde_json::from_str::<Value>(&stdout).unwrap()["data"].clone()
    };
    for (key, typed, kept) in [
        ("serve_insight", "1", json!(true)),
        ("serve_log_requests", "0", json!(false)),
        ("serve_insight", "TRUE", json!(true)),
        ("ai_insight_enabled", "on", json!(true)),
        ("no_progress", "yes", json!(true)),
        ("no_progress", "0", json!(false)),
        ("progress_delay_seconds", "7", json!(7)),
    ] {
        let (exit, stdout, stderr) = run_cli(&root, &["--json", "config", "set", key, typed]);
        assert_eq!(exit, 0, "{key}={typed}: {stdout}{stderr}");
        assert_eq!(get(key), kept, "{key}={typed}");
    }
    for (key, typed) in [
        ("serve_insight", "maybe"),
        ("no_progress", "maybe"),
        ("progress_delay_seconds", "soon"),
    ] {
        let (exit, stdout, _) = run_cli(&root, &["--json", "config", "set", key, typed]);
        let doc: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(exit, 2, "{key}={typed}: {stdout}");
        assert!(
            doc["error"]["message"].as_str().unwrap().contains(key),
            "{stdout}"
        );
    }

    // a file written before `set` checked the value: whichever way `no_progress` is written, it is read as `true`
    let path = root.join("config.json");
    for written in [json!(true), json!(1), json!("yes"), json!("on")] {
        std::fs::write(
            &path,
            json!({ "profiles": { "default": { "extra": { "no_progress": written } } } })
                .to_string(),
        )
        .unwrap();
        assert_eq!(get("no_progress"), json!(true), "{written}");
    }
}

/// A config file with a mistake in it is reported by the parser of its format, with the place of the mistake and
/// without the text there (#1044).
#[test]
fn a_config_file_with_a_mistake_is_reported_as_the_format_it_is_in() {
    let root = temp_root("config-mistake");
    let path = root.join("config.json");
    std::fs::write(
        &path,
        "{\n  \"current_profile\": \"default\",\n  \"profiles\": { \"default\": { \"wxid\": \"a\", } }\n}\n",
    )
    .unwrap();
    let (exit, stdout, _) = run_cli(&root, &["--json", "config", "list"]);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        (exit, &doc["error"]["code"]),
        (3, &json!("config_error")),
        "{stdout}"
    );
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(message.contains("line 3"), "{message}");
    assert!(!message.contains("TOML"), "{message}");

    // one line holding a key: the key is not in the message
    std::fs::write(
        &path,
        r#"{ "profiles": { "default": { "decrypt_key": "sEcr3tK3y", "image_xor_key": "abc", } } }"#,
    )
    .unwrap();
    let (exit, stdout, _) = run_cli(&root, &["--json", "config", "list"]);
    assert_eq!(exit, 3, "{stdout}");
    assert!(!stdout.contains("sEcr3tK3y"), "{stdout}");
}

#[test]
fn account_commands() {
    let root = temp_root("account");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);

    let r = server.call("GET", "/chat/sessions", "");
    let (_, stdout, _) = run_cli(&home, &["--json", "chat", "sessions"]);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.body, stdout);
    assert!(!r.json()["data"].as_array().unwrap().is_empty());
}

/// `image auto-download status` through `serve` is the server's own hook (here none: the tests turn it off), in the
/// document the command prints.
#[test]
fn the_auto_download_status_is_the_servers() {
    let root = temp_root("auto-download");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let r = server.call("GET", "/image/auto-download/status", "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.headers["x-weflow-exit-code"], "0");
    let (exit, stdout, _) = run_cli(&home, &["--json", "image", "auto-download", "status"]);
    assert_eq!(exit, 0);
    assert_eq!(r.body, stdout);
    let doc = r.json();
    assert_eq!(doc["data"]["isHooked"], false, "{doc}");
    assert_eq!(doc["data"]["pid"], Value::Null, "{doc}");
    assert!(doc["data"]["supported"].is_boolean(), "{doc}");
    // the command line it stands for, as for any command
    let r = server.call("GET", "/image/auto-download/status?std=in", "");
    assert_eq!(
        r.json()["argv"],
        json!(["weflow", "--json", "image", "auto-download", "status"])
    );
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

/// `sns export` takes its format in any case, and refuses another before it makes the folder (#1038).
#[test]
fn the_moments_export_reads_its_format_in_any_case() {
    let root = temp_root("sns-export-format");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let refused = root.join("refused");
    let body = json!({ "out": refused, "format": "bogus" });
    let r = server.call("POST", "/sns/export", &body.to_string());
    assert_eq!(r.status, 400, "{}", r.body);
    assert_eq!(r.json()["error"]["code"], "invalid_arguments");
    assert!(!refused.exists());

    let out = root.join("page");
    let (exit, stdout, stderr) = run_cli(
        &home,
        &[
            "--json",
            "sns",
            "export",
            "--format",
            "JSON",
            "--out",
            &out.to_string_lossy(),
        ],
    );
    assert_eq!(exit, 0, "{stdout}{stderr}");
    let answer: Value = serde_json::from_str(&stdout).unwrap();
    let file = PathBuf::from(answer["data"]["filePath"].as_str().unwrap());
    assert!(
        answer["data"]["postCount"].as_u64().unwrap() > 0 && file.starts_with(&out),
        "{answer}"
    );
    assert_eq!(file.extension().and_then(|e| e.to_str()), Some("json"));
}

/// The current time in Unix seconds.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Writes the test account's insight records file in `home`: one record of `wxid_bob` a `(id, seconds, millis)`.
fn write_insight_records(home: &Path, records: &[(&str, i64, i64)]) {
    let records: Vec<_> = records
        .iter()
        .map(
            |&(id, seconds, millis)| weflow_core::insight::InsightRecord {
                id: id.into(),
                account_scope: "wxid:wxid_me".into(),
                created_at: seconds * 1000 + millis,
                session_id: "wxid_bob".into(),
                trigger_reason: "activity".into(),
                insight: id.into(),
                ..Default::default()
            },
        )
        .collect();
    std::fs::create_dir_all(home.join("insights")).unwrap();
    std::fs::write(
        home.join("insights/wxid_me.json"),
        json!({ "version": 1, "records": records }).to_string(),
    )
    .unwrap();
}

/// `--start` and `--end` of `insight records` and `insight clear` are Unix seconds, the records' times milliseconds
/// (#1157): a day ahead lists and removes nothing, and `--end` takes the whole of its second.
#[test]
fn the_insight_records_are_picked_by_seconds() {
    let root = temp_root("insight-seconds");
    let home = account_home(&root);
    let now = unix_now();
    let day = 86_400;
    write_insight_records(&home, &[("now", now, 500), ("old", now - 2 * day, 0)]);
    let server = start(&home, &["--api-token", TOKEN]);
    let total = |query: &str| {
        let r = server.call("GET", &format!("/insight/records{query}"), "");
        assert_eq!(r.status, 200, "{query}: {}", r.body);
        r.json()["data"]["total"].as_i64().unwrap()
    };
    assert_eq!(total(""), 2);
    assert_eq!(total(&format!("?end={now}")), 2, "the whole of the second");
    assert_eq!(total(&format!("?end={}", now - day)), 1);
    assert_eq!(total(&format!("?start={}", now + day)), 0);
    assert_eq!(total(&format!("?start={}", now - day)), 1);

    let removed = |body: Value| {
        let r = server.call("DELETE", "/insight/clear", &body.to_string());
        assert_eq!(r.status, 200, "{body}: {}", r.body);
        r.json()["data"]["removed"].as_i64().unwrap()
    };
    assert_eq!(removed(json!({ "start": now + day, "yes": true })), 0);
    assert_eq!(removed(json!({ "end": now - day, "yes": true })), 1);
    assert_eq!(total(""), 1);
}

/// `insight clear` asks before it deletes (#1243): a server has no terminal to ask on, so without `yes` it refuses
/// with a usage error, `dry-run` counts and deletes nothing, and nothing to delete needs no `yes`.
#[test]
fn insight_clear_wants_yes() {
    let root = temp_root("insight-clear-yes");
    let home = account_home(&root);
    let now = unix_now();
    write_insight_records(&home, &[("a", now, 0), ("b", now - 60, 0)]);
    let server = start(&home, &["--api-token", TOKEN]);
    let clear = |body: Value| server.call("DELETE", "/insight/clear", &body.to_string());

    let refused = clear(json!({}));
    assert_eq!(refused.status, 400, "{}", refused.body);
    assert!(refused.body.contains("--yes"), "{}", refused.body);

    let plan = clear(json!({ "dry-run": true }));
    assert_eq!(plan.status, 200, "{}", plan.body);
    assert_eq!(plan.json()["data"], json!({ "dryRun": true, "records": 2 }));

    let nothing = clear(json!({ "session": "wxid_nobody" }));
    assert_eq!(nothing.status, 200, "{}", nothing.body);
    assert_eq!(nothing.json()["data"]["removed"], 0);

    let done = clear(json!({ "yes": true }));
    assert_eq!(done.status, 200, "{}", done.body);
    assert_eq!(done.json()["data"]["removed"], 2);
}

/// `insight today-stats` is answered by the server itself (#1264): the counts are kept by the insight engine in the
/// server's process, which a command in a child process would not see. A message newer than the server's start is
/// activity, the engine writes about it, and the route counts that.
#[test]
fn today_stats_reads_the_server_engine() {
    use weflow_native::fixture::{ContactSpec, Fixture, MsgSpec, SessionSpec};
    let fake = weflow_fake_model::FakeModel::start().unwrap();
    let root = temp_root("insight-today-stats");
    let fixture = Fixture::new(&root.path().join("data"), "wxid_me_ab12");
    // after the engine's start, which comes after this
    let later = unix_now() + 600;
    fixture.contact_db(
        &[
            ContactSpec::new("wxid_me", 1, "Me Nick"),
            ContactSpec::new("wxid_bob", 1, "Bob"),
        ],
        &[],
    );
    fixture.session_db(&[SessionSpec {
        username: "wxid_bob",
        summary: "news",
        last_timestamp: later,
        unread: 1,
        last_msg_type: 1,
    }]);
    fixture.message_shard(
        0,
        &[(
            "wxid_bob",
            vec![
                MsgSpec::text(1, "wxid_bob", later - 86_400, "yesterday"),
                MsgSpec::text(2, "wxid_bob", later, "news"),
            ],
        )],
    );
    let home = root.path().join("home");
    let base = fake.openai_base_url();
    let key_hex = fixture.key_hex();
    let db_path = fixture.account_dir.to_string_lossy().to_string();
    for (key, value) in [
        ("db_path", db_path.as_str()),
        ("wxid", "wxid_me_ab12"),
        ("decrypt_key", key_hex.as_str()),
        ("serve_image_auto_download", "false"),
        ("ai_model_api_base_url", base.as_str()),
        ("ai_model_api_key", weflow_fake_model::KEY),
        ("ai_model_api_model", "fake-chat"),
        ("ai_insight_enabled", "true"),
        ("ai_insight_filter_mode", "whitelist"),
        ("ai_insight_filter_list", r#"["wxid_bob"]"#),
        ("ai_insight_notification_enabled", "false"),
    ] {
        let (exit, _, stderr) = run_cli(&home, &["config", "set", key, value]);
        assert_eq!(exit, 0, "{key}: {stderr}");
    }
    let server = start(&home, &["--api-token", TOKEN, "--insight"]);

    // the engine looks every 5 s
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let written = loop {
        let r = server.call("GET", "/insight/records", "");
        assert_eq!(r.status, 200, "{}", r.body);
        if r.json()["data"]["total"].as_i64() == Some(1) {
            break r.json()["data"]["records"][0].clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no insight written: {:?}\n{}",
            fake.requests(),
            server.stderr.lock().unwrap().join("\n")
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    assert_eq!(written["sessionId"], "wxid_bob", "{written}");

    let r = server.call("GET", "/insight/today-stats", "");
    assert_eq!(r.status, 200, "{}", r.body);
    let stats = r.json()["data"].clone();
    let rows = stats.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{stats}");
    assert_eq!(rows[0]["sessionId"], "wxid_bob", "{stats}");
    assert_eq!(rows[0]["count"], 1, "{stats}");
}
