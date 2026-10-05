mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use serde_json::{json, Value};
use weflow_core::isaac64;
use weflow_core::services::{SnsExportOptions, SnsProxyResult};

/// Tiny HTTP server: path → (extra headers, body). Runs until the process exits.
fn serve(routes: Vec<(&'static str, Vec<(&'static str, &'static str)>, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let routes = Arc::new(routes);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let routes = routes.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let mut req = Vec::new();
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&req)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                match routes.iter().find(|(p, _, _)| path.starts_with(p)) {
                    Some((_, headers, body)) => {
                        let mut head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n",
                            body.len()
                        );
                        for (k, v) in headers {
                            head.push_str(&format!("{k}: {v}\r\n"));
                        }
                        head.push_str("\r\n");
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(body);
                    }
                    None => {
                        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            });
        }
    });
    format!("http://{addr}")
}

fn fake_jpeg() -> Vec<u8> {
    let mut v = vec![0xff, 0xd8, 0xff, 0xe0];
    v.extend((0..500u32).map(|i| (i % 251) as u8));
    v
}

fn encrypt(mut data: Vec<u8>, key: &str, limit: Option<usize>) -> Vec<u8> {
    isaac64::xor_in_place(&mut data, key, limit);
    data
}

#[test]
fn stats_usernames_and_caches() {
    let (hub, _root) = common::mock_hub("sns-stats");
    assert_eq!(
        hub.sns_usernames_list().unwrap(),
        vec!["wxid_bob", "wxid_carol", "wxid_me"]
    );
    let stats = hub.sns_export_stats(false).unwrap();
    assert_eq!(stats["totalPosts"], 5);
    assert_eq!(stats["totalFriends"], 3);
    assert_eq!(stats["myPosts"], 2);
    // fast path serves the cached value
    assert_eq!(hub.sns_export_stats(true).unwrap(), stats);
    let counts = hub.sns_user_post_counts(false).unwrap();
    assert_eq!(
        (counts["wxid_bob"], counts["wxid_carol"], counts["wxid_me"]),
        (2, 1, 2)
    );
    assert_eq!(
        hub.sns_user_post_stats("wxid_carol").unwrap()["totalPosts"],
        1
    );
    assert!(hub.sns_user_post_stats("  ").is_err());
}

#[test]
fn image_and_video_media_are_decrypted_and_cached() {
    let (hub, _root) = common::mock_hub("sns-media");
    let plain = fake_jpeg();
    let mut video_plain = vec![0, 0, 0, 0x18];
    video_plain.extend_from_slice(b"ftypmp42");
    video_plain.extend((0..400u32).map(|i| (i % 200) as u8));
    let base = serve(vec![
        (
            "/img",
            vec![("x-enc", "1"), ("Content-Type", "image/jpeg")],
            encrypt(plain.clone(), "123456", None),
        ),
        (
            "/plain",
            vec![("Content-Type", "image/jpeg")],
            plain.clone(),
        ),
        (
            "/snsvideodownload/clip",
            vec![],
            encrypt(video_plain.clone(), "99", Some(131072)),
        ),
    ]);
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let img = hub
            .sns_fetch_media(&format!("{base}/img"), Some("123456"))
            .await
            .unwrap();
        assert_eq!(img.data.as_deref(), Some(plain.as_slice()));
        assert_eq!(img.content_type, "image/jpeg");
        assert!(img.cache_path.as_ref().unwrap().exists());

        // unencrypted images pass through untouched
        let raw = hub
            .sns_fetch_media(&format!("{base}/plain"), None)
            .await
            .unwrap();
        assert_eq!(raw.data.as_deref(), Some(plain.as_slice()));

        // wrong key on an encrypted image is rejected
        let bad = hub
            .sns_fetch_media(&format!("{base}/img?bad=1"), Some("7"))
            .await;
        assert!(bad.is_err());

        let vid = hub
            .sns_fetch_media(&format!("{base}/snsvideodownload/clip"), Some("99"))
            .await
            .unwrap();
        assert_eq!(vid.content_type, "video/mp4");
        assert_eq!(vid.data.as_deref(), Some(video_plain.as_slice()));

        match hub
            .sns_proxy_image(&format!("{base}/img"), Some("123456"))
            .await
            .unwrap()
        {
            SnsProxyResult::DataUrl(d) => assert!(d.starts_with("data:image/jpeg;base64,")),
            other => panic!("unexpected {other:?}"),
        }
        match hub
            .sns_proxy_image(&format!("{base}/snsvideodownload/clip"), Some("99"))
            .await
            .unwrap()
        {
            SnsProxyResult::VideoPath(p) => assert!(p.unwrap().exists()),
            other => panic!("unexpected {other:?}"),
        }
    });
}

#[test]
fn emoji_download_decrypts_with_aes_key() {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes128Gcm, KeyInit, Nonce};
    let (hub, _root) = common::mock_hub("sns-emoji");
    let key = [3u8; 16];
    let nonce = [5u8; 12];
    let mut gif = b"GIF89a".to_vec();
    gif.extend(std::iter::repeat_n(9u8, 64));
    let sealed = Aes128Gcm::new_from_slice(&key)
        .unwrap()
        .encrypt(Nonce::from_slice(&nonce), gif.as_slice())
        .unwrap();
    let (ct, tag) = sealed.split_at(sealed.len() - 16);
    let mut enc = ct.to_vec();
    enc.extend_from_slice(&nonce);
    enc.extend_from_slice(tag);
    let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    let base = serve(vec![("/emoji.enc", vec![], enc)]);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let out = rt
        .block_on(hub.sns_download_emoji("", Some(&format!("{base}/emoji.enc")), Some(&hex)))
        .unwrap();
    let path = out["localPath"].as_str().unwrap().to_string();
    assert!(path.ends_with(".gif"));
    assert_eq!(std::fs::read(&path).unwrap(), gif);
    // second call hits the cache without any network
    let again = rt
        .block_on(hub.sns_download_emoji("http://unreachable.invalid/x", None, None))
        .is_err();
    assert!(again, "a different url is not cached");
}

#[test]
fn exports_json_html_and_arkmejson() {
    use weflow_native::fixture::{ContactSpec, SessionSpec, SnsPostSpec, T0};
    const RICH: &str = "<location city=\"Shanghai\" poiName=\"Bund\" latitude=\"31.2\" longitude=\"121.5\"/>\
        <comment_user_list>\
        <user_comment><comment_id>1</comment_id><username>wxid_carol</username><nickname>Carol</nickname><content>nice</content><ref_comment_id>0</ref_comment_id></user_comment>\
        <user_comment><comment_id>2</comment_id><username>wxid_bob</username><nickname>Bob</nickname><content>thx</content><ref_comment_id>1</ref_comment_id></user_comment>\
        </comment_user_list>";
    let (hub, root, _f) = common::custom_hub("sns-export", |f| {
        f.session_db(&[SessionSpec {
            username: "wxid_bob",
            summary: "",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 1,
        }]);
        f.contact_db(
            &[
                ContactSpec {
                    remark: "Bobby",
                    ..ContactSpec::new("wxid_bob", 1, "Bob")
                },
                ContactSpec::new("wxid_carol", 1, "Carol"),
            ],
            &[],
        );
        f.sns_db(&[
            SnsPostSpec {
                tid: 2001,
                user: "wxid_bob",
                create_time: T0 + 500,
                desc: "hello moments",
                kind: 1,
                media: 0,
                likes: &[("wxid_carol", "Carol")],
                extra: RICH,
            },
            SnsPostSpec {
                tid: 2002,
                user: "wxid_carol",
                create_time: T0 + 100,
                desc: "a video",
                kind: 15,
                media: 0,
                likes: &[],
                extra: "",
            },
        ]);
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut outputs = Vec::new();
    for fmt in ["json", "html", "arkmejson"] {
        let dir = root.join(format!("out-{fmt}"));
        let opts = SnsExportOptions {
            output_dir: dir.clone(),
            format: fmt.into(),
            ..Default::default()
        };
        let res = rt.block_on(hub.sns_export_timeline(&opts)).unwrap();
        assert_eq!(res["postCount"], 2);
        assert_eq!(res["mediaCount"], 0);
        let file = std::path::PathBuf::from(res["filePath"].as_str().unwrap());
        assert!(file.starts_with(&dir) && file.exists());
        outputs.push((fmt, std::fs::read_to_string(file).unwrap()));
    }
    let json: Value = serde_json::from_str(&outputs[0].1).unwrap();
    assert_eq!(json["totalPosts"], 2);
    assert_eq!(json["posts"][0]["nickname"], "Bobby");
    assert_eq!(json["posts"][0]["location"]["city"], "Shanghai");
    assert!(json["posts"][0]["createTimeStr"]
        .as_str()
        .unwrap()
        .contains('/'));

    assert!(outputs[1].1.contains("<title>朋友圈导出</title>"));
    assert!(outputs[1].1.contains("hello moments"));

    let ark: Value = serde_json::from_str(&outputs[2].1).unwrap();
    assert_eq!(ark["format"], "arkmejson");
    assert_eq!(ark["schemaVersion"], "1.0.0");
    assert_eq!(ark["posts"][0]["author"]["wxid"], "wxid_bob");
    assert_eq!(ark["posts"][0]["author"]["displayName"], "Bobby");
    assert_eq!(
        ark["posts"][0]["likesDetail"][0]["source"], "xml",
        "likes come from the post's like_user_list"
    );
    assert_eq!(
        ark["posts"][0]["commentsDetail"].as_array().unwrap().len(),
        2
    );
    assert_eq!(ark["mediaSelection"]["images"], false);
    let keys: Vec<&String> = ark["posts"][0].as_object().unwrap().keys().collect();
    let pos = |k: &str| keys.iter().position(|x| x.as_str() == k).unwrap();
    assert!(
        pos("author") == pos("nickname") + 1
            && pos("likesDetail") > pos("location")
            && pos("commentsDetail") == pos("likesDetail") + 1
    );
}

#[test]
fn debug_resource_reports_status_and_decryption_headers() {
    let (hub, _root) = common::mock_hub("sns-debug");
    let base = serve(vec![(
        "/res",
        vec![
            ("x-enc", "1"),
            ("x-time", "123"),
            ("Content-Type", "image/jpeg"),
        ],
        vec![1, 2, 3],
    )]);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let r = rt.block_on(hub.sns_debug_resource(&format!("{base}/res")));
    assert_eq!(r["success"], true, "{r}");
    assert_eq!(r["status"], 200);
    assert_eq!(
        (
            r["headers"]["x-enc"].clone(),
            r["headers"]["x-time"].clone(),
            r["headers"]["content-type"].clone()
        ),
        (json!("1"), json!("123"), json!("image/jpeg"))
    );
    assert_eq!(r["headers"]["content-length"], "3");
    assert_eq!(
        rt.block_on(hub.sns_debug_resource(&format!("{base}/missing")))["status"],
        404
    );
    assert_eq!(rt.block_on(hub.sns_debug_resource(" "))["success"], false);
    assert_eq!(
        rt.block_on(hub.sns_debug_resource("http://127.0.0.1:9/x"))["success"],
        false,
        "connection refused"
    );
}
