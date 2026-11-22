//! Input that cannot be used is refused as a usage error (#1047): `invalid_arguments`, HTTP 400 and exit code 2, from
//! the command line, from a route that runs the command in a child process and from the `/api/` route that runs it
//! in the server.

use serde_json::Value;

use super::common::{account_home, run_cli, start, temp_root, Reply, Server, TOKEN};

/// The reply is a 400 `invalid_arguments` whose message holds `said`.
fn refused(reply: &Reply, said: &str, what: &str) {
    assert_eq!(reply.status, 400, "{what}: {}", reply.body);
    let doc = reply.json();
    assert_eq!(
        doc["error"]["code"], "invalid_arguments",
        "{what}: {}",
        reply.body
    );
    let message = doc["error"]["message"].as_str().unwrap();
    assert!(message.contains(said), "{what}: {message}");
}

/// The command line ends with exit code 2 and says `said` on stderr.
fn refused_on_the_command_line(home: &std::path::Path, argv: &[&str], said: &str) {
    let (exit, stdout, stderr) = run_cli(home, argv);
    assert_eq!(exit, 2, "{argv:?}: {stdout}{stderr}");
    assert!(stderr.contains(said), "{argv:?}: {stderr}");
    // and as a `--json` document
    let mut json = vec!["--json"];
    json.extend_from_slice(argv);
    let (exit, stdout, _) = run_cli(home, &json);
    assert_eq!(exit, 2, "{json:?}: {stdout}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["error"]["code"], "invalid_arguments", "{json:?}");
}

/// A request of `route` with `query`, from the server as it is run in a child process and, with `api`, in the
/// server's own.
fn get_both(server: &Server, route: &str, query: &str, api: bool) -> Vec<(String, Reply)> {
    let mut targets = vec![format!("/{route}?{query}")];
    if api {
        targets.push(format!("/api/{route}?{query}"));
    }
    targets
        .into_iter()
        .map(|target| {
            let reply = server.call("GET", &target, "");
            (target, reply)
        })
        .collect()
}

/// Input a command refuses, as a request and as a command line, and what the refusal says.
struct NotUsable {
    route: &'static str,
    query: &'static str,
    argv: &'static [&'static str],
    said: &'static str,
    /// The command has an `/api/` route.
    api: bool,
}

const NOT_USABLE: &[NotUsable] = &[
    NotUsable {
        route: "chat/message",
        query: "session-id=wxid_bob&server-id=abc",
        argv: &["chat", "message", "wxid_bob", "--server-id", "abc"],
        said: "invalid server id: abc",
        api: true,
    },
    NotUsable {
        route: "chat/image-info",
        query: "session-id=wxid_bob&local-id=1",
        argv: &["chat", "image-info", "wxid_bob", "1"],
        said: "the message is not an image",
        api: true,
    },
    NotUsable {
        route: "chat/media-stream",
        query: "type=bogus",
        argv: &["chat", "media-stream", "--type", "bogus"],
        said: "unknown media type: bogus",
        api: false,
    },
    NotUsable {
        route: "chat/media-stream",
        query: "type=images",
        argv: &["chat", "media-stream", "--type=images"],
        said: "unknown media type: images",
        api: false,
    },
    NotUsable {
        route: "chat/resources",
        query: "type=bogus",
        argv: &["chat", "resources", "--type", "bogus"],
        said: "unknown resource type: bogus",
        api: true,
    },
    NotUsable {
        route: "chat/resources",
        query: "type=image&type=vedio",
        argv: &["chat", "resources", "--type", "image", "--type", "vedio"],
        said: "unknown resource type: vedio",
        api: true,
    },
];

#[test]
fn input_that_cannot_be_used_is_a_usage_error() {
    let root = temp_root("usage");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);

    for case in NOT_USABLE {
        for (target, reply) in get_both(&server, case.route, case.query, case.api) {
            refused(&reply, case.said, &target);
        }
        refused_on_the_command_line(&home, case.argv, case.said);
    }

    // a message of another kind: the file of an image or a voice message, through the `/api/` media route, a child
    // process that writes the file, and the command line
    let out = root.join("out.bin");
    let out = out.to_str().unwrap();
    for (name, said) in [
        ("image-data", "the message is not an image"),
        ("voice-data", "the message is not a voice message"),
    ] {
        let target = format!("/api/chat/{name}?session-id=wxid_bob&local-id=1");
        refused(&server.call("GET", &target, ""), said, &target);
        let child = server.call(
            "POST",
            &format!("/chat/{name}?return=file"),
            r#"{"session-id": "wxid_bob", "local-id": "1"}"#,
        );
        refused(&child, said, name);
        refused_on_the_command_line(&home, &["chat", name, "wxid_bob", "1", "--out", out], said);
    }
    assert!(!root.join("out.bin").exists());

    // what is not there is still not found, and an image that names no file still fails as a failure of the work
    let r = server.call("GET", "/chat/message?session-id=wxid_bob&server-id=77", "");
    assert_eq!(r.status, 404, "{}", r.body);
    let r = server.call(
        "GET",
        "/api/chat/image-info?session-id=wxid_bob&local-id=999",
        "",
    );
    assert_eq!(r.status, 404, "{}", r.body);
}

/// A command with counts or times that cannot be negative.
struct Counts {
    route: &'static str,
    /// The query a request needs besides the option, none for a command that needs nothing.
    needs: &'static str,
    /// The command line up to the options.
    argv: &'static [&'static str],
    options: &'static [&'static str],
    /// The command has an `/api/` route.
    api: bool,
}

const COUNTS: &[Counts] = &[
    Counts {
        route: "chat/sessions",
        needs: "",
        argv: &["chat", "sessions"],
        options: &["limit"],
        api: true,
    },
    Counts {
        route: "chat/messages",
        needs: "session-id=wxid_bob",
        argv: &["chat", "messages", "wxid_bob"],
        options: &["limit", "offset", "since"],
        api: true,
    },
    Counts {
        route: "chat/search",
        needs: "keyword=hello",
        argv: &["chat", "search", "hello"],
        options: &["limit", "offset", "start", "end"],
        api: true,
    },
    Counts {
        route: "chat/resources",
        needs: "",
        argv: &["chat", "resources"],
        options: &["limit"],
        api: true,
    },
    Counts {
        route: "chat/media-stream",
        needs: "",
        argv: &["chat", "media-stream"],
        options: &["limit", "offset"],
        api: false,
    },
    Counts {
        route: "sns/timeline",
        needs: "",
        argv: &["sns", "timeline"],
        options: &["limit", "offset", "start", "end"],
        api: true,
    },
    Counts {
        route: "biz/messages",
        needs: "username=wxid_bob",
        argv: &["biz", "messages", "wxid_bob"],
        options: &["limit", "offset"],
        api: true,
    },
    Counts {
        route: "biz/pay-records",
        needs: "",
        argv: &["biz", "pay-records"],
        options: &["limit", "offset"],
        api: true,
    },
    Counts {
        route: "group/ranking",
        needs: "chatroom-id=room1%40chatroom",
        argv: &["group", "ranking", "room1@chatroom"],
        options: &["limit"],
        api: true,
    },
    Counts {
        route: "group/member-messages",
        needs: "chatroom-id=room1%40chatroom&username=wxid_bob",
        argv: &["group", "member-messages", "room1@chatroom", "wxid_bob"],
        options: &["limit", "cursor"],
        api: true,
    },
    Counts {
        route: "analytics/rankings",
        needs: "",
        argv: &["analytics", "rankings"],
        options: &["limit"],
        api: true,
    },
    Counts {
        route: "insight/records",
        needs: "",
        argv: &["insight", "records"],
        options: &["limit", "offset", "start", "end"],
        api: false,
    },
];

#[test]
fn a_negative_count_or_time_is_a_usage_error() {
    let root = temp_root("usage-counts");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);

    for case in COUNTS {
        for option in case.options {
            let query = if case.needs.is_empty() {
                format!("{option}=-1")
            } else {
                format!("{}&{option}=-1", case.needs)
            };
            for (target, reply) in get_both(&server, case.route, &query, case.api) {
                refused(&reply, "-1 is not in 0..=", &target);
                // the option is named, as the command line names it
                assert!(reply.body.contains(&format!("--{option}")), "{target}");
            }
            // written after the option, which clap would take for an option of its own
            let flag = format!("--{option}");
            let mut line = case.argv.to_vec();
            line.extend([flag.as_str(), "-1"]);
            refused_on_the_command_line(&home, &line, "-1 is not in 0..=");
        }
    }

    // a value past the width of the number is refused the same way
    for (target, reply) in get_both(
        &server,
        "chat/messages",
        "session-id=wxid_bob&limit=2147483648",
        true,
    ) {
        refused(&reply, "is not in 0..=", &target);
    }

    // the count a group's hint is set to
    let r = server.call(
        "PUT",
        "/chat/group-hint",
        r#"{"chatroom-id": "room1@chatroom", "set": -1}"#,
    );
    refused(&r, "-1 is not in 0..=", "PUT /chat/group-hint");
    refused_on_the_command_line(
        &home,
        &["chat", "group-hint", "room1@chatroom", "--set", "-1"],
        "-1 is not in 0..=",
    );

    // a setting is refused by the config with its own message, from the command line as from the API
    refused_on_the_command_line(
        &home,
        &["config", "set", "progress_delay_seconds", "-1"],
        "progress_delay_seconds",
    );
    let r = server.call(
        "PUT",
        "/config/set",
        r#"{"key": "progress_delay_seconds", "value": "-1"}"#,
    );
    refused(&r, "progress_delay_seconds", "PUT /config/set");
}

/// A negative report year written after `--year` is a number to clap, not an option of its own, so the range of the
/// year refuses it (#1066) as it refuses the `--year=-5` spelling.
#[test]
fn a_negative_report_year_after_the_option_is_refused_by_its_range() {
    let root = temp_root("usage-year");
    let home = account_home(&root);
    for argv in [
        &["report", "annual", "generate", "--year", "-5"][..],
        &[
            "report", "dual", "generate", "--friend", "wxid_bob", "--year", "-5",
        ],
    ] {
        refused_on_the_command_line(&home, argv, "invalid year -5");
    }
}

/// How many items the answer of a route holds.
fn count(server: &Server, target: &str) -> usize {
    let reply = server.call("GET", target, "");
    assert_eq!(reply.status, 200, "{target}: {}", reply.body);
    let doc = reply.json();
    let data = &doc["data"];
    data.as_array()
        .or_else(|| data["timeline"].as_array())
        .unwrap_or_else(|| panic!("{target}: {data}"))
        .len()
}

#[test]
fn a_limit_of_zero_is_all_of_them() {
    let root = temp_root("usage-zero");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    // (the route, what else it needs, how many there are)
    for (route, needs, all) in [
        ("chat/sessions", "", 3),
        ("chat/messages", "session-id=wxid_bob&", 5),
        ("sns/timeline", "", 5),
        ("group/ranking", "chatroom-id=room1%40chatroom&", 3),
    ] {
        for prefix in ["", "/api"] {
            let of =
                |limit: usize| count(&server, &format!("{prefix}/{route}?{needs}limit={limit}"));
            assert_eq!(of(2), 2, "{prefix}/{route}");
            assert_eq!(of(0), all, "{prefix}/{route}");
        }
    }
    // the same for the ranking of private chats, with the one chat the account has
    for prefix in ["", "/api"] {
        assert_eq!(
            count(&server, &format!("{prefix}/analytics/rankings?limit=0")),
            1,
            "{prefix}"
        );
    }
}
