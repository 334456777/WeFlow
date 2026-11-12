//! Background jobs: running, cancelling, limits, and the files they wrote.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::common::{account_home, form_parts, run_cli, start, temp_root, TOKEN};

/// A `key db` job runs until it is cancelled (its stdin held open while it waits), then ends by itself well before
/// it would be killed: the hook it may have set is out of WeChat first (#287). It is cancelled while it waits for
/// WeChat, before any hook.
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn a_key_db_job_runs_until_cancelled_then_ends_by_itself() {
    let root = temp_root("key-db-job");
    let server = start(&root, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");
    let r = server.request(
        "POST",
        "/key/db",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        &json!({ "timeout": 60 }).to_string(),
    );
    assert_eq!(r.status, 202, "{}", r.body);
    let id = r.json()["data"]["id"].as_str().unwrap().to_string();
    std::thread::sleep(std::time::Duration::from_secs(3));
    let job = server.call("GET", &format!("/jobs/{id}"), "").json()["data"].clone();
    assert_eq!(job["status"], "running", "{job}");
    let asked = std::time::Instant::now();
    let r = server.call("DELETE", &format!("/jobs/{id}"), "");
    assert_eq!(r.status, 200, "{}", r.body);
    let job = loop {
        let job = server.call("GET", &format!("/jobs/{id}"), "").json()["data"].clone();
        if job["status"] != "running" {
            break job;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    assert_eq!(job["status"], "cancelled", "{job}");
    // killed only after 10 s; on its own it ends within about a second
    assert!(
        asked.elapsed() < std::time::Duration::from_secs(8),
        "{:?}",
        asked.elapsed()
    );
}

/// The events of a server-sent event stream: (name, data).
fn sse_events(body: &str) -> Vec<(String, Value)> {
    body.split("\n\n")
        .filter_map(|block| {
            let mut name = None;
            let mut data = None;
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("event: ") {
                    name = Some(v.to_string());
                } else if let Some(v) = line.strip_prefix("data: ") {
                    data = Some(serde_json::from_str(v).unwrap());
                }
            }
            Some((name?, data?))
        })
        .collect()
}

#[test]
fn jobs_run_in_the_background() {
    let root = temp_root("jobs");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");
    let out = root.join("backup.zip");
    let body = json!({ "out": out }).to_string();

    let r = server.request(
        "POST",
        "/backup/create",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        &body,
    );
    assert_eq!(r.status, 202, "{}", r.body);
    let job = r.json()["data"].clone();
    let id = job["id"].as_str().unwrap().to_string();
    assert_eq!(r.headers["location"], format!("/jobs/{id}"));
    assert!(matches!(job["status"].as_str(), Some("queued" | "running")));
    assert_eq!(
        job["command"].as_array().unwrap()[..3],
        json!(["weflow", "--json", "--progress"])
            .as_array()
            .unwrap()[..]
    );

    // the events: status, progress from `--progress`, and the result at the end
    let r = server.call("GET", &format!("/jobs/{id}/events"), "");
    assert_eq!(r.status, 200);
    assert!(r.headers["content-type"].starts_with("text/event-stream"));
    let events = sse_events(&r.body);
    let names: Vec<&str> = events.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names.first(), Some(&"status"), "{names:?}");
    assert!(names.contains(&"progress"), "{names:?}");
    let (last, result) = events.last().unwrap();
    assert_eq!(last, "result");
    assert_eq!(result["status"], "succeeded", "{result}");
    assert_eq!(result["result"]["success"], true);
    assert!(out.is_file());

    // the job keeps its result; a late subscriber still gets every event
    let r = server.call("GET", &format!("/jobs/{id}"), "");
    assert_eq!(r.status, 200);
    let doc = r.json()["data"].clone();
    assert_eq!(
        (
            doc["status"].as_str(),
            doc["exitCode"].as_i64(),
            doc["httpStatus"].as_u64()
        ),
        (Some("succeeded"), Some(0), Some(200))
    );
    assert_eq!(doc["result"], result["result"]);
    let again = sse_events(&server.call("GET", &format!("/jobs/{id}/events"), "").body);
    assert_eq!(again.len(), events.len());
    // after `Last-Event-ID` only the newer ones
    let r = server.request(
        "GET",
        &format!("/jobs/{id}/events"),
        &[("Authorization", &auth), ("Last-Event-ID", "1")],
        "",
    );
    assert_eq!(sse_events(&r.body).len(), events.len() - 1);

    let list = server.call("GET", "/jobs", "").json();
    assert_eq!(list["data"][0]["id"], id.as_str());
    assert!(list["data"][0].get("result").is_none());

    // a failing command is a failed job with the command's error
    let r = server.request(
        "GET",
        "/config/get?key=a&key=b",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    let failed = r.json()["data"]["id"].as_str().unwrap().to_string();
    let events = sse_events(
        &server
            .call("GET", &format!("/jobs/{failed}/events"), "")
            .body,
    );
    let (_, result) = events.last().unwrap();
    assert_eq!(result["status"], "failed", "{result}");
    assert_eq!(result["result"]["success"], false);

    // DELETE forgets a finished job
    assert_eq!(
        server.call("DELETE", &format!("/jobs/{id}"), "").status,
        200
    );
    assert_eq!(server.call("GET", &format!("/jobs/{id}"), "").status, 404);
    assert_eq!(server.call("GET", "/jobs/nope/events", "").status, 404);
    let r = server.call("POST", "/jobs", "");
    assert_eq!((r.status, r.headers["allow"].as_str()), (405, "GET"));
    // the human-readable answers are not jobs
    let r = server.request(
        "GET",
        "/config/path?std=out",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    assert_eq!(r.status, 400);
}

#[test]
fn files_come_back_with_the_result() {
    let root = temp_root("files");
    let home = account_home(&root);
    // files of an earlier run are cleared when the server starts
    let stale = home.join("cache").join("serve-files").join("old");
    std::fs::create_dir_all(&stale).unwrap();
    let server = start(&home, &["--api-token", TOKEN]);
    assert!(!stale.exists());

    // no --out: the server picks the folder, the command names the file
    let r = server.call("POST", "/export/sessions?return=file", "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.headers["content-type"].starts_with("multipart/form-data; boundary="));
    assert_eq!(
        r.headers["content-length"].parse::<usize>().unwrap(),
        r.bytes.len()
    );
    let parts = form_parts(&r);
    assert_eq!(parts.len(), 2);
    assert!(parts[0].0.contains("name=\"result\""), "{}", parts[0].0);
    let result: Value = serde_json::from_slice(&parts[0].1).unwrap();
    assert_eq!(result["success"], true);
    let path = PathBuf::from(result["data"]["path"].as_str().unwrap());
    assert_eq!(path.file_name().unwrap(), "sessions.json");
    assert!(
        path.starts_with(home.join("cache").join("serve-files")),
        "{path:?}"
    );
    assert!(
        parts[1].0.contains("filename=\"sessions.json\""),
        "{}",
        parts[1].0
    );
    assert!(
        parts[1].0.contains("Content-Type: application/json"),
        "{}",
        parts[1].0
    );
    assert_eq!(parts[1].1, std::fs::read(&path).unwrap());

    // with --out the file is written there and sent as well
    let out = root.join("mine.zip");
    let r = server.call(
        "POST",
        "/backup/create?return=file",
        &json!({ "out": out }).to_string(),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let parts = form_parts(&r);
    let result: Value = serde_json::from_slice(&parts[0].1).unwrap();
    assert_eq!(
        std::fs::canonicalize(result["data"]["path"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(&out).unwrap()
    );
    assert!(
        parts[1].0.contains("Content-Type: application/zip"),
        "{}",
        parts[1].0
    );
    assert_eq!(parts[1].1, std::fs::read(&out).unwrap());

    // the folder is part of `cache list` (`--api`)
    let (_, stdout, _) = run_cli(&home, &["--json", "cache", "list"]);
    let list: Value = serde_json::from_str(&stdout).unwrap();
    let api = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["part"] == "api")
        .unwrap()
        .clone();
    assert!(api["paths"].to_string().contains("serve-files"), "{api}");
    assert!(api["files"].as_u64().unwrap() >= 1);

    // a failure is the usual JSON error
    let r = server.call(
        "POST",
        "/export/messages?return=file",
        r#"{"session-id": "x", "format": "nope"}"#,
    );
    assert_eq!(r.status, 400, "{}", r.body);
    assert_eq!(r.json()["success"], false);
    // only commands that write one file
    for (method, target) in [
        ("GET", "/chat/sessions?return=file"),
        ("POST", "/export/media?return=file"),
        ("POST", "/export/sessions?return=file&std=out"),
        ("POST", "/export/sessions?return=zip"),
    ] {
        let r = server.call(method, target, "");
        assert_eq!(r.status, 400, "{target}: {}", r.body);
    }
}

#[test]
fn jobs_keep_the_file_they_wrote_for_the_browser() {
    let root = temp_root("job-files");
    let home = account_home(&root);
    let server = start(&home, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");
    let async_call = |target: &str| {
        let r = server.request(
            "POST",
            target,
            &[("Authorization", &auth), ("Prefer", "respond-async")],
            "",
        );
        assert_eq!(r.status, 202, "{target}: {}", r.body);
        r.json()["data"]["id"].as_str().unwrap().to_string()
    };
    let finished = |id: &str| {
        let events = sse_events(&server.call("GET", &format!("/jobs/{id}/events"), "").body);
        events.last().unwrap().1.clone()
    };

    // the server picks the folder, as for a direct request, and keeps the file once the job has succeeded
    let id = async_call("/export/sessions?return=file");
    let done = finished(&id);
    assert_eq!(done["status"], "succeeded", "{done}");
    assert_eq!(done["file"]["name"], "sessions.json");
    let path = PathBuf::from(done["result"]["data"]["out"].as_str().unwrap());
    assert!(
        path.starts_with(home.join("cache").join("serve-files")),
        "{path:?}"
    );
    let listed = server.call("GET", "/jobs", "").json();
    assert_eq!(
        listed["data"][0]["file"]["size"],
        std::fs::metadata(&path).unwrap().len()
    );

    let r = server.call("GET", &format!("/jobs/{id}/file"), "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.headers["content-type"].starts_with("application/json"));
    assert!(r.headers["content-disposition"].starts_with("attachment; filename=\"sessions.json\""));
    assert_eq!(r.bytes, std::fs::read(&path).unwrap());
    assert_eq!(
        server.call("POST", &format!("/jobs/{id}/file"), "").status,
        405
    );

    // forgetting the job removes the folder the server chose
    assert_eq!(
        server.call("DELETE", &format!("/jobs/{id}"), "").status,
        200
    );
    assert!(!path.parent().unwrap().exists());

    // several formats packed with --zip (#342): the job's file is the zip, the formats' files inside it
    let id = async_call(
        "/export/messages?return=file&session-id=wxid_bob&format=html&format=txt&zip=true",
    );
    let done = finished(&id);
    assert_eq!(done["status"], "succeeded", "{done}");
    assert_eq!(done["file"]["name"], "wxid_bob.zip", "{done}");
    let r = server.call("GET", &format!("/jobs/{id}/file"), "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(
        r.headers["content-type"].starts_with("application/zip"),
        "{:?}",
        r.headers
    );
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(r.bytes)).unwrap();
    let mut names: Vec<String> = archive.file_names().map(str::to_string).collect();
    names.sort();
    assert_eq!(names, ["wxid_bob.html", "wxid_bob.txt"]);
    let mut text = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("wxid_bob.txt").unwrap(), &mut text)
        .unwrap();
    assert!(text.contains("see you"), "{text}");
    let zipped = PathBuf::from(done["result"]["data"]["out"].as_str().unwrap());
    assert!(
        !zipped.with_file_name(".wxid_bob.zip.parts").exists(),
        "the folder the zip was made from is taken away"
    );
    assert_eq!(done["result"]["data"]["files"][1]["out"], "wxid_bob.txt");
    assert_eq!(
        server.call("DELETE", &format!("/jobs/{id}"), "").status,
        200
    );

    // a job without return=file, or one that failed, has no file
    let plain = async_call("/export/sessions");
    assert_eq!(finished(&plain)["file"], Value::Null);
    assert_eq!(
        server
            .call("GET", &format!("/jobs/{plain}/file"), "")
            .status,
        404
    );
    let failed = async_call("/export/messages?return=file&session-id=x&format=nope");
    assert_eq!(finished(&failed)["status"], "failed");
    assert_eq!(
        server
            .call("GET", &format!("/jobs/{failed}/file"), "")
            .status,
        404
    );
    // its folder is gone
    let left: Vec<_> = std::fs::read_dir(home.join("cache").join("serve-files"))
        .map(|d| d.flatten().collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
}

/// An AI endpoint that accepts connections and never answers; reports when each one opens and closes.
fn silent_ai_server() -> (String, std::sync::mpsc::Receiver<&'static str>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let tx = tx.clone();
            let _ = tx.send("open");
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while matches!(stream.read(&mut buf), Ok(n) if n > 0) {}
                let _ = tx.send("closed");
            });
        }
    });
    (url, rx)
}

#[test]
fn jobs_are_cancelled_and_limited() {
    use std::time::Duration;
    let root = temp_root("cancel");
    let (url, ai) = silent_ai_server();
    for (key, value) in [
        ("ai_model_api_base_url", url.as_str()),
        ("ai_model_api_key", "k"),
    ] {
        let (exit, _, stderr) = run_cli(&root, &["config", "set", key, value]);
        assert_eq!(exit, 0, "{stderr}");
    }
    let server = start(&root, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");
    let queue = |target: &str| {
        let r = server.request(
            "POST",
            target,
            &[("Authorization", &auth), ("Prefer", "respond-async")],
            "",
        );
        assert_eq!(r.status, 202, "{}", r.body);
        r.json()["data"]["id"].as_str().unwrap().to_string()
    };
    // `insight test` waits for the AI server (15 s at most)
    let first = queue("/insight/test");
    assert_eq!(ai.recv_timeout(Duration::from_secs(30)), Ok("open"));

    // one task at a time: another one is refused right away, or queued
    let r = server.call("POST", "/insight/test", "");
    assert_eq!(r.status, 429, "{}", r.body);
    assert_eq!(r.headers["retry-after"], "5");
    assert_eq!(r.json()["error"]["code"], "busy");
    let second = queue("/insight/test");
    let r = server.call("GET", &format!("/jobs/{second}"), "");
    assert_eq!(r.json()["data"]["status"], "queued");
    // reads are not held up by tasks
    assert_eq!(server.call("GET", "/config/path", "").status, 200);

    // cancelling kills the command: its connection to the AI server goes away
    let r = server.call("DELETE", &format!("/jobs/{first}"), "");
    assert_eq!(r.status, 200);
    let doc = r.json()["data"].clone();
    assert_eq!(doc["status"], "cancelled", "{doc}");
    assert_eq!(doc["httpStatus"], 499);
    assert_eq!(doc["result"]["error"]["code"], "user_interrupt");
    assert_eq!(ai.recv_timeout(Duration::from_secs(5)), Ok("closed"));
    // and the queued job starts
    assert_eq!(ai.recv_timeout(Duration::from_secs(30)), Ok("open"));
    let r = server.call("DELETE", &format!("/jobs/{second}"), "");
    assert_eq!(r.json()["data"]["status"], "cancelled");
    assert_eq!(ai.recv_timeout(Duration::from_secs(5)), Ok("closed"));

    // a client that goes away takes its command with it
    let mut stream = TcpStream::connect(&server.addr).unwrap();
    write!(
        stream,
        "POST /insight/test HTTP/1.1\r\nHost: {}\r\nAuthorization: {auth}\r\nContent-Length: 0\r\n\r\n",
        server.addr
    )
    .unwrap();
    assert_eq!(ai.recv_timeout(Duration::from_secs(30)), Ok("open"));
    drop(stream);
    assert_eq!(ai.recv_timeout(Duration::from_secs(5)), Ok("closed"));
}

/// Cancelling a job twice: the first time cancels it, the second forgets it; neither changes how it ended.
#[test]
fn a_job_is_cancelled_once_and_then_forgotten() {
    use std::time::Duration;
    let root = temp_root("cancel-twice");
    let (url, ai) = silent_ai_server();
    for (key, value) in [
        ("ai_model_api_base_url", url.as_str()),
        ("ai_model_api_key", "k"),
    ] {
        let (exit, _, stderr) = run_cli(&root, &["config", "set", key, value]);
        assert_eq!(exit, 0, "{stderr}");
    }
    let server = start(&root, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");
    let r = server.request(
        "POST",
        "/insight/test",
        &[("Authorization", &auth), ("Prefer", "respond-async")],
        "",
    );
    assert_eq!(r.status, 202, "{}", r.body);
    let id = r.json()["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(ai.recv_timeout(Duration::from_secs(30)), Ok("open"));

    let r = server.call("DELETE", &format!("/jobs/{id}"), "");
    assert_eq!(r.status, 200);
    let cancelled = r.json()["data"].clone();
    assert_eq!(cancelled["status"], "cancelled", "{cancelled}");
    assert_eq!(ai.recv_timeout(Duration::from_secs(5)), Ok("closed"));
    // its events end with the result, once
    let events = sse_events(&server.call("GET", &format!("/jobs/{id}/events"), "").body);
    let results: Vec<&Value> = events
        .iter()
        .filter(|(n, _)| n == "result")
        .map(|(_, d)| d)
        .collect();
    assert_eq!(results.len(), 1, "{events:?}");
    assert_eq!(results[0]["status"], "cancelled");
    assert_eq!(events.last().unwrap().0, "result");

    let r = server.call("DELETE", &format!("/jobs/{id}"), "");
    assert_eq!(r.status, 200);
    let forgotten = r.json()["data"].clone();
    assert_eq!(
        (
            &forgotten["status"],
            &forgotten["finishedAt"],
            &forgotten["result"]
        ),
        (
            &cancelled["status"],
            &cancelled["finishedAt"],
            &cancelled["result"]
        )
    );
    assert_eq!(server.call("GET", &format!("/jobs/{id}"), "").status, 404);
    assert_eq!(
        server.call("DELETE", &format!("/jobs/{id}"), "").status,
        404
    );
}
