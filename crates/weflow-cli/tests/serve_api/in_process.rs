//! Reads run in the server's process: the same answers and failures as in a child, compression and `?fields=`,
//! `Server-Timing`, and a profile changed in between.

use std::io::Read;
use std::path::PathBuf;

use serde_json::{json, Value};
use weflow_native::fixture::Fixture;

use super::common::{account, account_home, account_home_with, run_cli, start, temp_root, TOKEN};

#[test]
fn reads_run_in_process_with_the_same_answers() {
    let root = temp_root("in-process");
    let home = account_home_with(&root, |f| {
        use weflow_native::fixture::{SnsPostSpec, T0};
        let post = |tid: i64, user: &'static str, desc: &'static str| SnsPostSpec {
            tid,
            user,
            create_time: T0 + tid * 60,
            desc,
            kind: 1,
            media: tid as usize % 3,
            likes: &[("wxid_me_ab12", "Me")],
            extra: "",
        };
        f.sns_db(&[
            post(1, "wxid_bob", "first"),
            post(2, "wxid_me_ab12", "mine"),
            post(3, "wxid_bob", "third"),
        ]);
    });
    let server = start(&home, &["--api-token", TOKEN]);
    let same = |target: &str| {
        let child = server.call("GET", target, "");
        let own = server.call("GET", &format!("/api{target}"), "");
        assert_eq!(
            (own.status, &own.body),
            (child.status, &child.body),
            "{target}"
        );
        for h in ["x-weflow-exit-code", "x-weflow-error-code"] {
            assert_eq!(own.headers.get(h), child.headers.get(h), "{target} {h}");
        }
        own
    };
    for target in [
        "/chat/sessions",
        "/chat/sessions?limit=1",
        "/chat/messages?session-id=wxid_bob&limit=2&offset=1",
        "/chat/messages?session-id=room1%40chatroom",
        "/chat/latest?session-id=wxid_bob&limit=1",
        "/chat/message?session-id=wxid_bob&local-id=2",
        "/chat/message?session-id=wxid_bob&local-id=2&parsed=true",
        "/chat/contact?username=wxid_bob",
        "/chat/contacts",
        "/chat/counts?sessions=wxid_bob&sessions=room1%40chatroom",
        "/chat/search?keyword=hello",
        "/chat/dates?session-id=wxid_bob",
        "/chat/date-counts?session-id=wxid_bob",
        "/chat/detail?session-id=wxid_bob&fast=true",
        "/chat/resources",
        "/chat/resources?session=wxid_bob&type=image&limit=1",
        // the rest of the details, and the timeline with its media, from the child process
        "/chat/detail?session-id=wxid_bob",
        "/sns/timeline",
        "/sns/timeline?limit=1&offset=1&user=wxid_bob",
        "/sns/timeline?keyword=thi",
        "/sns/users",
        "/sns/stats",
        "/sns/stats?fast=true",
        "/sns/post-counts",
        "/sns/post-counts?prefer-cache=true",
        "/sns/post-counts?user=wxid_bob",
        // #413: the other reads the WebUI asks for
        "/chat/statuses?usernames=wxid_bob&usernames=room1%40chatroom",
        "/chat/statuses",
        "/chat/tab-counts",
        "/group/list",
        "/group/ranking?chatroom-id=room1%40chatroom",
        "/group/ranking?chatroom-id=room1%40chatroom&limit=1&start=2020-01-01&end=2099-12-31",
        "/group/hours?chatroom-id=room1%40chatroom",
        "/group/media?chatroom-id=room1%40chatroom",
        "/group/member?chatroom-id=room1%40chatroom&username=wxid_bob",
        "/group/member-messages?chatroom-id=room1%40chatroom&username=wxid_bob&limit=1",
        "/group/member-messages?chatroom-id=room1%40chatroom&username=wxid_bob&cursor=1",
        "/analytics/overall",
        "/analytics/rankings",
        "/analytics/rankings?limit=1&start=2020-01-01",
        "/analytics/time",
        "/analytics/excluded",
        "/analytics/exclude-candidates",
        "/biz/accounts",
        "/biz/messages?username=wxid_bob&limit=2",
        "/biz/pay-records",
        "/report/annual/years",
        "/cache/list",
        "/config/get",
        "/config/get?key=wxid",
        "/config/get?key=config_path",
        "/config/profiles",
        "/runtime/info",
        "/db/detect",
        "/db/test",
        "/db/wxid",
    ] {
        let r = same(target);
        assert_eq!(r.status, 200, "{target}: {}", r.body);
    }
    // an insight record that is not there: the command's own failure
    assert_eq!(same("/insight/get?id=none").status, 500);
    // a cursor that is not one fails the same way
    assert_eq!(same("/chat/resources?before=1.2.3").status, 400);
    // A media list lookup has exactly the parsed shape and cursor of the conversation page.
    let parsed = same("/chat/messages?session-id=wxid_bob&parsed=true").json();
    let lookup = same("/chat/message?session-id=wxid_bob&local-id=2&parsed=true").json();
    assert_eq!(
        parsed["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["localId"] == 2),
        Some(&lookup["data"])
    );
    // a page from a message's cursor: the one before Bob's message 4 (in message_1.db) goes on in message_0.db
    {
        use weflow_native::fixture::{DAY, T0};
        let latest = same("/chat/messages?session-id=wxid_bob&limit=2").json();
        let cursor = format!("{}.{}.4.1", (T0 + 3 * DAY) * 1000, T0 + 3 * DAY);
        assert_eq!(latest["data"][1]["_cursor"], cursor.as_str());
        let r = same(&format!(
            "/chat/messages?session-id=wxid_bob&limit=2&before={cursor}"
        ));
        assert_eq!(r.status, 200, "{}", r.body);
        let ids: Vec<i64> = r.json()["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["localId"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, [3, 2]);
        assert_eq!(
            same(&format!(
                "/chat/messages?session-id=wxid_bob&before={cursor}&offset=1"
            ))
            .status,
            400
        );
    }
    // the footprint says how long it took
    for target in [
        "/insight/footprint",
        "/insight/footprint?start=2020-01-01&end=2099-12-31",
    ] {
        let timed = |path: &str| {
            let r = server.call("GET", path, "");
            assert_eq!(r.status, 200, "{path}: {}", r.body);
            let mut doc = r.json();
            assert!(doc["data"]["diagnostics"]["elapsed_ms"].take().is_number());
            doc
        };
        assert_eq!(timed(&format!("/api{target}")), timed(target), "{target}");
    }
    // the member panel carries the time it was made, and is built anew every time
    let target = "/group/members?chatroom-id=room1%40chatroom";
    let panel = |path: &str| {
        let r = server.call("GET", path, "");
        assert_eq!(r.status, 200, "{path}: {}", r.body);
        let mut doc = r.json();
        assert!(doc["data"].get("fromCache").is_none());
        doc["data"]["updatedAt"].take();
        doc
    };
    assert_eq!(panel(&format!("/api{target}")), panel(target));
    assert_eq!(
        server
            .call("GET", &format!("/api{target}&refresh=true"), "")
            .status,
        400,
        "refresh is no parameter of the route"
    );
    // failures answer the same too: from the command, and from parsing (left to the child process)
    assert_eq!(same("/chat/message?session-id=wxid_bob").status, 400);
    assert_eq!(same("/chat/messages").status, 400);
    assert_eq!(
        same("/chat/messages?limit=many&session-id=wxid_bob").status,
        400
    );

    // a changed config is read again
    let r = server.call(
        "PUT",
        "/config/set",
        r#"{"key": "decrypt_key", "value": "00"}"#,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_ne!(same("/chat/sessions").status, 200);

    // only the listed reads, as plain --json
    let r = server.call("POST", "/api/export/sessions", "");
    assert_eq!(r.status, 404);
    assert_eq!(server.call("GET", "/api/config/list", "").status, 404);
    assert_eq!(server.call("POST", "/api/chat/sessions", "").status, 405);
    for target in ["/api/chat/sessions?std=out", "/api/chat/sessions?std=in"] {
        assert_eq!(server.call("GET", target, "").status, 400, "{target}");
    }
    let auth = format!("Bearer {TOKEN}");
    let r = server.request(
        "GET",
        "/api/chat/sessions",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    assert_eq!(r.status, 400);
}

#[test]
fn answers_are_compressed_when_asked_and_cut_to_fields() {
    let root = temp_root("reply-size");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");

    // gzip or br as the client takes them, the same answer once decoded; nothing without `Accept-Encoding`
    for target in ["/chat/contacts", "/api/chat/contacts"] {
        let plain = server.call("GET", target, "");
        assert_eq!(plain.status, 200, "{}", plain.body);
        assert_eq!(plain.headers.get("content-encoding"), None);
        let ask = |encoding: &str| {
            server.request(
                "GET",
                target,
                &[("Authorization", &auth), ("Accept-Encoding", encoding)],
                "",
            )
        };
        let gz = ask("gzip");
        assert_eq!(gz.headers["content-encoding"], "gzip", "{target}");
        let mut unzipped = Vec::new();
        flate2::read::GzDecoder::new(gz.bytes.as_slice())
            .read_to_end(&mut unzipped)
            .unwrap();
        assert_eq!(unzipped, plain.bytes, "{target}");
        let br = ask("br;q=1, gzip;q=0.5");
        assert_eq!(br.headers["content-encoding"], "br", "{target}");
        let mut unbrotlied = Vec::new();
        brotli_decompressor::Decompressor::new(br.bytes.as_slice(), 4096)
            .read_to_end(&mut unbrotlied)
            .unwrap();
        assert_eq!(unbrotlied, plain.bytes, "{target}");
        assert_eq!(ask("identity").headers.get("content-encoding"), None);
    }

    // `fields`: only those keys of each row, the same from both routes, however the names are given
    let rows = |target: &str| {
        let child = server.call("GET", target, "");
        let own = server.call("GET", &format!("/api{target}"), "");
        assert_eq!(
            (own.status, &own.body),
            (child.status, &child.body),
            "{target}"
        );
        assert_eq!(child.status, 200, "{target}: {}", child.body);
        child.json()["data"].clone()
    };
    let page = rows("/chat/messages?session-id=wxid_bob&fields=localId,createTime");
    let full = rows("/chat/messages?session-id=wxid_bob");
    assert!(full[0].as_object().unwrap().len() > 2);
    let cut: Vec<Value> = full
        .as_array()
        .unwrap()
        .iter()
        .map(|m| json!({ "localId": m["localId"], "createTime": m["createTime"] }))
        .collect();
    assert_eq!(page, Value::Array(cut));
    assert_eq!(
        rows("/chat/messages?session-id=wxid_bob&fields=localId&fields=+createTime,,"),
        page
    );
    let contacts = rows("/chat/contacts?fields=username,nosuchfield");
    assert!(contacts.as_array().unwrap().iter().all(|c| c
        .as_object()
        .unwrap()
        .keys()
        .eq(["username"])));
    // a failure as it is
    let failed = server.call("GET", "/chat/messages?fields=local_id", "");
    assert_eq!(
        (failed.status, &failed.body),
        (400, &server.call("GET", "/chat/messages", "").body)
    );
    for target in [
        "/chat/sessions?fields=username&std=out",
        "/chat/sessions?fields=username&return=file",
    ] {
        assert_eq!(server.call("GET", target, "").status, 400, "{target}");
    }
    let r = server.request(
        "GET",
        "/chat/sessions?fields=username",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    assert_eq!(r.status, 400);
}

/// An `/api/` request with parameters the command line refuses gets clap's error from the server's process, the one
/// the route without `/api/` answers from a child process, without starting one (#432).
#[test]
fn usage_errors_are_answered_in_process() {
    let root = temp_root("usage-errors");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    for target in [
        "/chat/messages",
        "/chat/messages?session-id=wxid_bob&limit=many",
        "/chat/messages?session-id=wxid_bob&limit=1&limit=2",
        "/sns/timeline?limit=x",
        "/group/members",
    ] {
        let own = server.call("GET", &format!("/api{target}"), "");
        let child = server.call("GET", target, "");
        assert_eq!(
            (own.status, &own.body),
            (child.status, &child.body),
            "{target}"
        );
        assert_eq!(own.status, 400, "{target}: {}", own.body);
        assert_eq!(own.json()["error"]["code"], "invalid_arguments");
        for h in ["x-weflow-exit-code", "x-weflow-error-code"] {
            assert_eq!(own.headers.get(h), child.headers.get(h), "{target} {h}");
        }
        let timing = &own.headers["server-timing"];
        assert!(
            timing.contains("cmd;desc=\"refused in process\""),
            "{target}: {timing}"
        );
    }
}

#[test]
fn answers_tell_where_the_time_went() {
    let root = temp_root("timing");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN, "--log-requests"]);
    // `Server-Timing`: what ran the command and how long it took, then the total
    let stages = |target: &str| {
        let r = server.call("GET", target, "");
        assert_eq!(r.status, 200, "{target}: {}", r.body);
        let timing = r.headers["server-timing"].clone();
        timing
            .split(", ")
            .map(|s| {
                let (name, rest) = s.split_once(';').unwrap();
                let ms: f64 = rest.rsplit_once("dur=").unwrap().1.parse().unwrap();
                assert!(ms >= 0.0, "{timing}");
                let desc = rest
                    .split_once("desc=\"")
                    .map(|(_, d)| d.split('"').next().unwrap().to_string());
                (name.to_string(), desc)
            })
            .collect::<Vec<_>>()
    };
    let cmd = |desc: &str| ("cmd".to_string(), Some(desc.to_string()));
    let total = ("total".to_string(), None);
    assert_eq!(
        stages("/api/chat/sessions?limit=1"),
        [
            ("wait".to_string(), Some("in line for a place".to_string())),
            cmd("in process"),
            total.clone()
        ]
    );
    assert_eq!(
        stages("/chat/sessions?limit=1"),
        [cmd("child process"), total.clone()]
    );
    assert_eq!(stages("/health"), [total]);
    let local = server.request("GET", "/health", &[("Origin", "http://localhost:5173")], "");
    assert_eq!(
        local.headers["timing-allow-origin"],
        "http://localhost:5173"
    );
    assert!(local.headers["access-control-expose-headers"].contains("Server-Timing"));

    // `--log-requests`: one line each, with the path and without the query, which may hold ids
    let r = server.call("GET", "/chat/messages?session-id=wxid_bob&limit=1", "");
    assert_eq!(r.status, 200);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let logged = loop {
        let lines = server.stderr.lock().unwrap().clone();
        let found: Vec<Value> = lines
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|e| e["type"] == "request")
            .collect();
        if found.len() >= 5 || std::time::Instant::now() > deadline {
            break (found, lines);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let (events, lines) = logged;
    assert!(lines.iter().all(|l| !l.contains("wxid_bob")), "{lines:?}");
    let last = events.last().unwrap_or_else(|| panic!("{lines:?}"));
    assert_eq!(last["method"], "GET");
    assert_eq!(last["path"], "/chat/messages");
    assert_eq!(last["status"], 200);
    assert!(last["ms"].as_f64().unwrap() >= 0.0);
    assert_eq!(events.len(), 5, "{lines:?}");
}

// The regression matrix of the refactoring in #391 (#403): behaviour the shared business entry point (#397) and the
// job and HTTP layers (#398) must keep; the scenarios and the baseline are in #403.

/// A failed read answers the same through `/api/` as through the child process: status, body and error headers, for
/// the ways a request goes wrong before, while and after its command runs.
#[test]
fn reads_fail_the_same_in_process_and_in_a_child() {
    let root = temp_root("errors");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let same = |method: &str, target: &str, headers: &[(&str, &str)]| {
        let child = server.request(method, target, headers, "");
        let own = server.request(method, &format!("/api{target}"), headers, "");
        assert_eq!(
            (own.status, &own.body),
            (child.status, &child.body),
            "{method} {target}"
        );
        for h in ["x-weflow-exit-code", "x-weflow-error-code", "allow"] {
            assert_eq!(
                own.headers.get(h),
                child.headers.get(h),
                "{method} {target} {h}"
            );
        }
        own
    };
    let auth = format!("Bearer {TOKEN}");
    let with_token: &[(&str, &str)] = &[("Authorization", &auth)];
    for target in [
        // the command line is refused: a required argument missing, a value of the wrong type, arguments that
        // exclude each other
        "/chat/messages",
        "/chat/messages?session-id=wxid_bob&limit=many",
        "/chat/messages?session-id=wxid_bob&offset=1&before=1",
        // the server refuses a name the command does not have, and a switch that is neither true nor false
        "/chat/messages?session-id=wxid_bob&bogus=1",
        "/chat/sessions?parsed=maybe",
        // the command itself refuses
        "/sns/timeline?limit=x",
        "/chat/resources?before=1.2.3",
    ] {
        let r = same("GET", target, with_token);
        assert_eq!(r.status, 400, "{target}: {}", r.body);
        assert_eq!(r.json()["success"], false, "{target}");
        assert_eq!(r.json()["error"]["code"], "invalid_arguments", "{target}");
    }
    // nothing found is a success with nothing in it
    for target in [
        "/chat/messages?session-id=wxid_nobody",
        "/chat/search?keyword=qzxwvk",
        "/chat/date-counts?session-id=wxid_nobody",
    ] {
        let r = same("GET", target, with_token);
        assert_eq!(r.status, 200, "{target}: {}", r.body);
    }
    // the method, and the token: refused before any command runs
    let r = same("POST", "/chat/sessions", with_token);
    assert_eq!((r.status, r.headers["allow"].as_str()), (405, "GET"));
    for headers in [&[][..], &[("Authorization", "Bearer wrong")][..]] {
        let r = same("GET", "/chat/sessions", headers);
        assert_eq!(r.status, 401);
        assert_eq!(r.json()["error"]["code"], "unauthorized");
        let r = server.request(
            "GET",
            "/api/chat/image-data?session-id=wxid_bob&msg-id=1",
            headers,
            "",
        );
        assert_eq!(r.status, 401);
    }
    // what `/api/` does not take
    for target in [
        "/api/chat/sessions?std=in",
        "/api/chat/sessions?return=file",
    ] {
        assert_eq!(server.call("GET", target, "").status, 400, "{target}");
    }
    let r = server.request(
        "GET",
        "/api/chat/sessions",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    assert_eq!(r.status, 400);
    let r = server.call("GET", "/api/config/path", "");
    assert_eq!(r.status, 404);
    assert!(r.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("/config/path"));
}

/// Another profile, another account: the reads of both routes answer from the account the config names now, right
/// after it changed, also when the names of the profiles are as long as each other (the hub tells a changed config by
/// the time and size of its file).
#[test]
fn reads_follow_the_profile_right_after_it_changes() {
    use weflow_native::fixture::{MsgSpec, SessionSpec, T0};
    let root = temp_root("profiles");
    let (home, first) = account(&root);
    // the second account: Carol only
    let other = Fixture::standard(&root.join("data-b"));
    other.session_db(&[SessionSpec {
        username: "wxid_carol",
        summary: "from b",
        last_timestamp: T0,
        unread: 0,
        last_msg_type: 1,
    }]);
    other.message_shard(
        0,
        &[(
            "wxid_carol",
            vec![MsgSpec::text(1, "wxid_carol", T0, "only in b")],
        )],
    );
    let profile = |name: &str, account: &Fixture| {
        for (key, value) in [
            ("current_profile", name.to_string()),
            ("db_path", account.account_dir.to_string_lossy().to_string()),
            ("wxid", "wxid_me_ab12".to_string()),
            ("decrypt_key", account.key_hex()),
        ] {
            let (exit, _, stderr) = run_cli(&home, &["config", "set", key, &value]);
            assert_eq!(exit, 0, "{stderr}");
        }
    };
    profile("pb", &other);
    // the first account again, under a name as long as the second's
    profile("pa", &first);
    let server = start(&home, &["--api-token", TOKEN]);
    let sessions = |route: &str| -> Vec<String> {
        let r = server.call("GET", &format!("{route}/chat/sessions"), "");
        assert_eq!(r.status, 200, "{}", r.body);
        r.json()["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["username"].as_str().unwrap().to_string())
            .collect()
    };
    let start_list = sessions("/api");
    assert!(start_list.iter().any(|s| s == "wxid_bob"), "{start_list:?}");
    assert_eq!(sessions(""), start_list);
    for round in 0..4 {
        let (name, in_b) = if round % 2 == 0 {
            ("pb", true)
        } else {
            ("pa", false)
        };
        let r = server.call(
            "PUT",
            "/config/set",
            &json!({ "key": "current_profile", "value": name }).to_string(),
        );
        assert_eq!(r.status, 200, "{}", r.body);
        // at once, without waiting for anything
        let own = sessions("/api");
        assert_eq!(
            own.iter().any(|s| s == "wxid_carol"),
            in_b,
            "round {round} ({name}): {own:?}"
        );
        assert_eq!(own.iter().any(|s| s == "wxid_bob"), !in_b, "round {round}");
        assert_eq!(sessions(""), own, "round {round}");
        let r = server.call("GET", "/api/chat/messages?session-id=wxid_carol", "");
        let found = r.json()["data"].as_array().map_or(0, Vec::len);
        assert_eq!(found > 0, in_b, "round {round}: {}", r.body);
    }

    // another config file chosen (#413): its profile at once, and the config reads say so
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("config.json")).unwrap()).unwrap();
    let mut moved = config;
    moved["current_profile"] = json!("pb");
    let other = root.join("other").join("config.json");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    std::fs::write(&other, moved.to_string()).unwrap();
    let path = other.to_string_lossy().to_string();
    let r = server.call(
        "PUT",
        "/config/set",
        &json!({ "key": "config_path", "value": path }).to_string(),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let own = sessions("/api");
    assert!(own.iter().any(|s| s == "wxid_carol"), "{own:?}");
    assert_eq!(sessions(""), own);
    for route in ["/api", ""] {
        let r = server.call("GET", &format!("{route}/config/get?key=config_path"), "");
        // as paths: `config set` makes the path absolute, which on Windows also writes every separator as `\`,
        // while the target folder the test runs in may be written with `/` (#655)
        let read = r.json()["data"].as_str().map(PathBuf::from);
        assert_eq!(read.as_deref(), Some(other.as_path()), "{route}");
    }
    let r = server.call(
        "DELETE",
        "/config/unset",
        &json!({ "key": "config_path" }).to_string(),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let own = sessions("/api");
    assert!(own.iter().any(|s| s == "wxid_bob"), "{own:?}");
    assert_eq!(sessions(""), own);
}
