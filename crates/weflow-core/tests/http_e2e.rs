#![cfg(target_os = "linux")]
mod common;

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use weflow_core::http_server::{serve, HttpConfig};
use weflow_core::push::PushBroker;
use weflow_core::services::ServiceHub;

const TOKEN: &str = "s3cret";

struct Server {
    base: String,
    broker: Arc<PushBroker>,
    hub: ServiceHub,
    client: reqwest::Client,
    _root: std::path::PathBuf,
}

async fn start(tag: &str, token: Option<&str>, push: bool) -> Server {
    let (hub, root) = common::mock_hub(tag);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let broker = PushBroker::new();
    let cfg = HttpConfig { host: "127.0.0.1".into(), port: addr.port(), token: token.map(str::to_string), push_enabled: push };
    tokio::spawn(serve(hub.clone(), cfg, broker.clone(), listener));
    Server { base: format!("http://{addr}"), broker, hub, client: reqwest::Client::new(), _root: root }
}

fn auth(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    req.bearer_auth(TOKEN)
}

#[tokio::test]
async fn health_needs_no_token_everything_else_does() {
    let s = start("http-auth", Some(TOKEN), false).await;
    for path in ["/health", "/api/v1/health"] {
        let r = s.client.get(format!("{}{path}", s.base)).send().await.unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(r.json::<Value>().await.unwrap(), json!({"status": "ok"}));
    }
    let r = s.client.get(format!("{}/api/v1/sessions", s.base)).send().await.unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(r.json::<Value>().await.unwrap()["error"], "Unauthorized: Invalid or missing access_token");
    let r = s.client.get(format!("{}/api/v1/sessions", s.base)).bearer_auth("nope").send().await.unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(auth(s.client.get(format!("{}/api/v1/sessions", s.base))).send().await.unwrap().status(), 200);
    assert_eq!(s.client.get(format!("{}/api/v1/sessions?access_token={TOKEN}", s.base)).send().await.unwrap().status(), 200);
    let r = s.client.post(format!("{}/api/v1/sessions", s.base)).json(&json!({"access_token": TOKEN})).send().await.unwrap();
    assert_eq!(r.status(), 200, "tokens may travel in a POST body");
}

#[tokio::test]
async fn no_configured_token_refuses_everything() {
    let s = start("http-notoken", None, false).await;
    let r = auth(s.client.get(format!("{}/api/v1/sessions", s.base))).send().await.unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(s.client.get(format!("{}/health", s.base)).send().await.unwrap().status(), 200);
}

#[tokio::test]
async fn query_routes_return_the_desktop_shapes() {
    let s = start("http-routes", Some(TOKEN), false).await;
    let get = |p: &str| auth(s.client.get(format!("{}{p}", s.base)));
    let sessions: Value = get("/api/v1/sessions?keyword=bob").send().await.unwrap().json().await.unwrap();
    assert_eq!(sessions["count"], 1);
    let messages: Value = get("/api/v1/messages?talker=wxid_bob&limit=2").send().await.unwrap().json().await.unwrap();
    assert_eq!(messages["count"], 2);
    assert_eq!(messages["hasMore"], true);
    let missing = get("/api/v1/messages").send().await.unwrap();
    assert_eq!(missing.status(), 400);
    assert_eq!(missing.json::<Value>().await.unwrap()["error"], "Missing required parameter: talker");
    let pull: Value = get("/api/v1/sessions/wxid_bob/messages?limit=1").send().await.unwrap().json().await.unwrap();
    assert_eq!(pull["sync"]["hasMore"], true);
    let contacts: Value = get("/api/v1/contacts?keyword=carol").send().await.unwrap().json().await.unwrap();
    assert_eq!(contacts["count"], 1);
    let members: Value = get("/api/v1/group-members?chatroomId=room1@chatroom").send().await.unwrap().json().await.unwrap();
    assert_eq!(members["count"], 3);
    assert_eq!(get("/api/v1/nope").send().await.unwrap().status(), 404);
}

#[tokio::test]
async fn cors_and_method_checks() {
    let s = start("http-cors", Some(TOKEN), false).await;
    let r = s.client.request(reqwest::Method::OPTIONS, format!("{}/api/v1/sessions", s.base)).header("Origin", "http://localhost:3000").send().await.unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(r.headers()["access-control-allow-origin"], "http://localhost:3000");
    assert_eq!(r.headers()["access-control-allow-methods"], "GET, POST, DELETE, OPTIONS");
    let r = s.client.request(reqwest::Method::OPTIONS, format!("{}/api/v1/sessions", s.base)).header("Origin", "http://evil.example").send().await.unwrap();
    assert!(r.headers().get("access-control-allow-origin").is_none(), "only local origins are echoed");
    let r = auth(s.client.post(format!("{}/api/v1/sns/timeline", s.base))).send().await.unwrap();
    assert_eq!(r.status(), 405);
    assert_eq!(r.headers()["allow"], "GET");
    let r = auth(s.client.get(format!("{}/api/v1/sns/export", s.base))).send().await.unwrap();
    assert_eq!(r.status(), 405);
    assert_eq!(r.headers()["allow"], "POST");
}

#[tokio::test]
async fn sns_routes() {
    let s = start("http-sns", Some(TOKEN), false).await;
    let get = |p: &str| auth(s.client.get(format!("{}{p}", s.base)));
    let t: Value = get("/api/v1/sns/timeline?limit=5").send().await.unwrap().json().await.unwrap();
    assert_eq!(t["success"], true);
    assert_eq!(t["count"], 2);
    let m = &t["timeline"][0]["media"][0];
    assert!(m["proxyUrl"].as_str().unwrap().starts_with(&format!("{}/api/v1/sns/media/proxy?url=", s.base)));
    assert_eq!(m["rawUrl"], "https://mmsns.qpic.cn/a/0?token=TK&idx=1");
    assert_eq!(m["url"], m["resolvedUrl"], "replace defaults to true");
    let plain: Value = get("/api/v1/sns/timeline?media=0").send().await.unwrap().json().await.unwrap();
    assert!(plain["timeline"][0]["media"][0].get("proxyUrl").is_none());
    let users: Value = get("/api/v1/sns/usernames").send().await.unwrap().json().await.unwrap();
    assert_eq!(users["usernames"], json!(["wxid_bob", "wxid_carol"]));
    let stats: Value = get("/api/v1/sns/export/stats?fast=1").send().await.unwrap().json().await.unwrap();
    assert_eq!(stats["data"]["totalPosts"], 2);
    assert_eq!(get("/api/v1/sns/media/proxy").send().await.unwrap().status(), 400);
    let status: Value = get("/api/v1/sns/block-delete/status").send().await.unwrap().json().await.unwrap();
    assert_eq!(status["success"], true);
    let install: Value = auth(s.client.post(format!("{}/api/v1/sns/block-delete/install", s.base))).send().await.unwrap().json().await.unwrap();
    assert_eq!(install["success"], true);
    let del = auth(s.client.delete(format!("{}/api/v1/sns/post/123", s.base))).send().await.unwrap();
    assert_eq!(del.status(), 200);

    let dir = s._root.join("export-out");
    let res = auth(s.client.post(format!("{}/api/v1/sns/export", s.base))).json(&json!({"outputDir": dir, "format": "arkme-json"})).send().await.unwrap();
    assert_eq!(res.status(), 200);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["postCount"], 2);
    assert!(std::path::Path::new(body["filePath"].as_str().unwrap()).exists());
    let bad = auth(s.client.post(format!("{}/api/v1/sns/export", s.base))).json(&json!({"outputDir": dir, "format": "pdf"})).send().await.unwrap();
    assert_eq!(bad.status(), 400);
    let no_dir = auth(s.client.post(format!("{}/api/v1/sns/export", s.base))).json(&json!({})).send().await.unwrap();
    assert_eq!(no_dir.status(), 400);
}

#[tokio::test]
async fn media_files_are_served_inside_the_export_dir_only() {
    let s = start("http-media", Some(TOKEN), false).await;
    let dir = s.hub.api_media_dir().join("sess/images");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.png"), b"\x89PNGdata").unwrap();
    std::fs::write(s.hub.api_media_dir().join("clip.mp4"), b"mp4").unwrap();
    let get = |p: &str| auth(s.client.get(format!("{}{p}", s.base)));
    let r = get("/api/v1/media/sess/images/a.png").send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "image/png");
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"\x89PNGdata");
    assert_eq!(get("/api/v1/media/clip.mp4").send().await.unwrap().headers()["content-type"], "video/mp4");
    assert_eq!(get("/api/v1/media/sess/images/none.png").send().await.unwrap().status(), 404);
    let escape = s.client.get(format!("{}/api/v1/media/..%2F..%2Fconfig.json", s.base)).bearer_auth(TOKEN).send().await.unwrap();
    assert_eq!(escape.status(), 403);
}

#[tokio::test]
async fn push_stream_ready_events_and_replay() {
    let s = start("http-push", Some(TOKEN), true).await;
    s.broker.broadcast(&json!({"event": "message.new", "sessionId": "wxid_bob", "content": "early"}));
    let mut r = auth(s.client.get(format!("{}/api/v1/push/messages", s.base))).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
    let mut text = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !text.contains("early") && tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(chunk))) = tokio::time::timeout(Duration::from_millis(500), r.chunk()).await {
            text.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
    assert!(text.contains("event: ready"), "{text}");
    assert!(text.contains("id: 1\nevent: message.new"), "buffered events are replayed: {text}");
    s.broker.broadcast(&json!({"event": "message.revoke", "sessionId": "wxid_bob", "content": "late"}));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !text.contains("late") && tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(chunk))) = tokio::time::timeout(Duration::from_millis(500), r.chunk()).await {
            text.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
    assert!(text.contains("id: 2\nevent: message.revoke"), "{text}");
    drop(r);

    // reconnect after id 1: only the newer event is replayed
    let mut r2 = s.client.get(format!("{}/api/v1/push/messages", s.base)).bearer_auth(TOKEN).header("Last-Event-ID", "1").send().await.unwrap();
    let mut text2 = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !text2.contains("late") && tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(chunk))) = tokio::time::timeout(Duration::from_millis(500), r2.chunk()).await {
            text2.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
    assert!(!text2.contains("early") && text2.contains("late"), "{text2}");

    let off = start("http-push-off", Some(TOKEN), false).await;
    let r = auth(off.client.get(format!("{}/api/v1/push/messages", off.base))).send().await.unwrap();
    assert_eq!(r.status(), 403);
    assert_eq!(r.json::<Value>().await.unwrap()["error"], "Message push is disabled");
}
