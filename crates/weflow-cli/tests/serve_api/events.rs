//! Event streams: `/health/events` and the push of new messages.

use std::io::{Read, Write};
use std::net::TcpStream;

use super::common::{account, start, start_env, temp_root, Server, TOKEN};

struct EventStream {
    stream: TcpStream,
    text: String,
}

impl EventStream {
    fn open(server: &Server, headers: &[(&str, &str)]) -> Self {
        let auth = format!("Bearer {TOKEN}");
        Self::open_at(
            server,
            "/api/push/messages",
            &[&[("Authorization", auth.as_str())], headers].concat(),
        )
    }

    fn open_at(server: &Server, path: &str, headers: &[(&str, &str)]) -> Self {
        let mut stream = TcpStream::connect(&server.addr).unwrap();
        let mut req = format!("GET {path} HTTP/1.1\r\nHost: {}\r\n", server.addr);
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes()).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        Self {
            stream,
            text: String::new(),
        }
    }

    /// Reads until `needle` arrives (true) or `secs` pass (false).
    fn wait_for(&mut self, needle: &str, secs: u64) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        let mut buf = [0u8; 4096];
        while !self.text.contains(needle) && std::time::Instant::now() < deadline {
            match self.stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.text.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(_) => {}
            }
        }
        self.text.contains(needle)
    }
}

#[test]
fn health_events_say_hello_then_beat_without_a_token() {
    let root = temp_root("health-events");
    let server = start_env(
        &root,
        &["--api-token", TOKEN],
        &[("WEFLOW_SSE_HEARTBEAT_MS", "200")],
    );
    let health = server.request("GET", "/health", &[], "").json();
    let mut events = EventStream::open_at(&server, "/health/events", &[]);
    assert!(events.wait_for("event: heartbeat", 10), "{}", events.text);
    assert!(events.text.starts_with("HTTP/1.1 200"), "{}", events.text);
    assert!(events.text.contains("text/event-stream"), "{}", events.text);
    // hello first, with what /health says; then heartbeats with the data
    let hello = events.text.find("event: hello").unwrap();
    assert!(hello < events.text.find("event: heartbeat").unwrap());
    for key in ["apiVersion", "version", "data"] {
        let said = format!("\"{key}\":{}", health[key]);
        assert!(events.text.contains(&said), "{key}: {}", events.text);
    }
    assert!(events.text.contains(&format!(
        "event: heartbeat\ndata: {{\"data\":{}}}",
        health["data"]
    )));
    let beats = events.text.matches("event: heartbeat").count();
    events.wait_for("not in the stream", 1);
    assert!(events.text.matches("event: heartbeat").count() >= beats + 2);

    assert_eq!(
        server.request("POST", "/health/events", &[], "").status,
        405
    );
    // the Host check holds here too
    let mut stream = TcpStream::connect(&server.addr).unwrap();
    stream
        .write_all(
            b"GET /health/events HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
}

// Ctrl+C as a signal; Windows has no way to send one to another process alone
#[cfg(unix)]
#[test]
fn event_streams_say_bye_when_the_server_stops() {
    use std::process::Command;

    let root = temp_root("bye");
    let server = start(&root, &["--api-token", TOKEN]);
    let mut events = EventStream::open_at(&server, "/health/events", &[]);
    assert!(events.wait_for("event: hello", 10), "{}", events.text);
    let sent = Command::new("kill")
        .args(["-INT", &server.child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    assert!(events.wait_for("event: bye", 5), "{}", events.text);
    assert!(
        events.text.contains("\"reason\":\"stopped\""),
        "{}",
        events.text
    );
}

#[test]
fn new_messages_are_pushed_as_events() {
    use weflow_native::fixture::{MsgSpec, SessionSpec, DAY, T0};
    let root = temp_root("push");
    let (home, fixture) = account(&root);
    let server = start(&home, &["--api-token", TOKEN]);

    // as a browser asks: the events stream as they come, not compressed
    let mut first = EventStream::open(&server, &[("Accept-Encoding", "gzip, deflate, br")]);
    assert!(first.wait_for("event: ready", 10), "{}", first.text);
    assert!(
        !first.text.to_ascii_lowercase().contains("content-encoding"),
        "{}",
        first.text
    );
    assert!(first.text.starts_with("HTTP/1.1 200"), "{}", first.text);
    assert!(first.text.contains("text/event-stream"), "{}", first.text);
    // the engine takes its baseline within a second of the first listener
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Bob writes again: a newer conversation time and a new message in the second message database
    fixture.message_shard(
        1,
        &[(
            "wxid_bob",
            vec![
                MsgSpec::text(4, "wxid_me", T0 + 3 * DAY, "later, compressed").compressed(),
                MsgSpec::text(5, "wxid_bob", T0 + 4 * DAY, "see you"),
                MsgSpec::text(6, "wxid_bob", T0 + 5 * DAY, "brand new"),
            ],
        )],
    );
    fixture.session_db(&[
        SessionSpec {
            username: "wxid_bob",
            summary: "brand new",
            last_timestamp: T0 + 5 * DAY,
            unread: 3,
            last_msg_type: 1,
        },
        SessionSpec {
            username: "room1@chatroom",
            summary: "ok",
            last_timestamp: T0 + 2 * DAY,
            unread: 0,
            last_msg_type: 1,
        },
        SessionSpec {
            username: "gh_news",
            summary: "daily",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 49,
        },
    ]);
    assert!(first.wait_for("brand new", 10), "{}", first.text);
    assert!(first.text.contains("event: message.new"), "{}", first.text);
    assert!(
        first.text.contains(r#""sessionId":"wxid_bob""#),
        "{}",
        first.text
    );
    let id: u64 = first
        .text
        .lines()
        .find_map(|l| l.strip_prefix("id: "))
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    // a page that comes later gets the kept events; one that reconnects after them does not get them again
    let mut later = EventStream::open(&server, &[]);
    assert!(later.wait_for("brand new", 5), "{}", later.text);
    let last = id.to_string();
    let mut again = EventStream::open(&server, &[("Last-Event-ID", &last)]);
    assert!(again.wait_for("event: ready", 5), "{}", again.text);
    assert!(!again.wait_for("brand new", 2), "{}", again.text);
    // one that comes back with the id of a run before (from 1 on, say) misses none of this run's: they start at
    // the time the server started
    assert!(id > 1_700_000_000_000, "{id}");
    let mut restarted = EventStream::open(&server, &[("Last-Event-ID", "57")]);
    assert!(restarted.wait_for("brand new", 5), "{}", restarted.text);
    // nothing new: each message is pushed once
    assert!(
        !first.wait_for(&format!("id: {}", id + 1), 2),
        "{}",
        first.text
    );

    let r = server.call("POST", "/api/push/messages", "");
    assert_eq!(r.status, 405);
    assert_eq!(
        server.request("GET", "/api/push/messages", &[], "").status,
        401
    );
}
