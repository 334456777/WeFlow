//! Moments on a synthetic account: the counts, the media fetched from a local server and decrypted, stickers and the
//! exports.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};
use weflow_core::isaac64;
use weflow_core::services::{Freshness, SnsExportOptions, SnsProxyResult, SnsTimelineQuery};

/// (path, extra headers, body)
type Route = (&'static str, Vec<(&'static str, &'static str)>, Vec<u8>);

/// Tiny HTTP server: path → (extra headers, body). Runs until the process exits.
fn serve(routes: Vec<Route>) -> String {
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
    let (_root, hub) = common::mock_hub("sns-stats");
    assert_eq!(
        hub.sns_usernames_list().unwrap(),
        vec!["wxid_bob", "wxid_carol", "wxid_me"]
    );
    let stats = hub.sns_export_stats(Freshness::Fresh).unwrap();
    assert_eq!(stats.total_posts, 5);
    assert_eq!(stats.total_friends, 3);
    assert_eq!(stats.my_posts, Some(2));
    // fast path serves the cached value
    assert_eq!(hub.sns_export_stats(Freshness::Cached).unwrap(), stats);
    let counts = hub.sns_user_post_counts(Freshness::Fresh).unwrap();
    assert_eq!(
        (counts["wxid_bob"], counts["wxid_carol"], counts["wxid_me"]),
        (2, 1, 2)
    );
    assert_eq!(
        hub.sns_user_post_stats("wxid_carol").unwrap().total_posts,
        1
    );
    assert!(hub.sns_user_post_stats("  ").is_err());
}

#[test]
fn image_and_video_media_are_decrypted_and_cached() {
    let (_root, hub) = common::mock_hub("sns-media");
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
fn media_the_timeline_cannot_fetch_point_at_the_route_of_sns_media() {
    let (_root, hub) = common::mock_hub("sns-with-media");
    let base = serve(vec![]);
    let mut posts = hub
        .sns_timeline_query(&SnsTimelineQuery {
            limit: 1,
            ..SnsTimelineQuery::default()
        })
        .unwrap();
    let missing = format!("{base}/missing?a=1");
    posts[0].media =
        vec![
            serde_json::from_value(json!({ "url": missing, "thumb": "", "key": " 42 " })).unwrap(),
        ];
    let rt = tokio::runtime::Runtime::new().unwrap();
    let posts = rt.block_on(hub.sns_enrich_timeline_media(posts));
    let media = &posts[0].media[0];
    // `weflow serve` answers `sns media` at /api/sns/media (#663)
    let proxy = format!(
        "/api/sns/media?url={}&key=42",
        missing
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' =>
                    (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    );
    assert_eq!(media.proxy_url.as_deref(), Some(proxy.as_str()));
    assert_eq!(media.resolved_url.as_deref(), Some(proxy.as_str()));
    assert_eq!(media.url, proxy);
    assert_eq!(media.raw_url.as_deref(), Some(missing.as_str()));
    // an empty address has nothing to fetch or proxy
    assert_eq!(media.proxy_thumb_url, None);
}

#[test]
fn emoji_download_decrypts_with_aes_key() {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes128Gcm, KeyInit, Nonce};
    let (_root, hub) = common::mock_hub("sns-emoji");
    let key = [3u8; 16];
    let nonce = [5u8; 12];
    let mut gif = b"GIF89a".to_vec();
    gif.extend(std::iter::repeat_n(9u8, 64));
    let sealed = Aes128Gcm::new_from_slice(&key)
        .unwrap()
        .encrypt(&Nonce::from(nonce), gif.as_slice())
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
    let path = out.local_path;
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("gif"));
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
    let (root, hub, _f) = common::custom_hub("sns-export", |f| {
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
        assert_eq!((res.post_count, res.media_count), (2, 0));
        let file = res.file_path;
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

    // the page's data (#705): the posts newest first, with their place and the comments, a reply naming whom it answers
    let html = &outputs[1].1;
    let open = r#"<script id="wf-data" type="application/json">"#;
    let start = html.find(open).unwrap() + open.len();
    let end = start + html[start..].find("</script>").unwrap();
    let page: Value = serde_json::from_str(&html[start..end]).unwrap();
    let posts = page["posts"].as_array().unwrap();
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0]["text"], "hello moments");
    let bobby = page["users"]
        .as_array()
        .unwrap()
        .iter()
        .position(|u| u[0] == "Bobby")
        .unwrap();
    assert_eq!(posts[0]["u"], bobby);
    assert!(
        posts[0]["loc"].as_str().unwrap().contains("Bund"),
        "{}",
        posts[0]
    );
    assert_eq!(posts[0]["likes"], serde_json::json!(["Carol"]));
    assert_eq!(posts[0]["cmts"][0], serde_json::json!(["Carol", "nice"]));
    assert_eq!(
        posts[0]["cmts"][1],
        serde_json::json!(["Bob", "thx", "Carol"])
    );

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

/// Exports to JSON an account whose Moments are `(tid, create_time)`, returning the exported post ids.
fn export_posts_with_times(tag: &str, times: &[(i64, i64)]) -> Vec<String> {
    use weflow_native::fixture::SnsPostSpec;
    let (root, hub, _f) = common::custom_hub(tag, |f| {
        let specs: Vec<SnsPostSpec> = times
            .iter()
            .map(|&(tid, create_time)| SnsPostSpec {
                tid,
                user: "wxid_bob",
                create_time,
                desc: "post",
                kind: 1,
                media: 0,
                likes: &[],
                extra: "",
            })
            .collect();
        f.session_db(&[]);
        f.contact_db(&[], &[]);
        f.sns_db(&specs);
    });
    let opts = SnsExportOptions {
        output_dir: root.join("out"),
        format: "json".into(),
        ..Default::default()
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let res = rt.block_on(hub.sns_export_timeline(&opts)).unwrap();
    let json: Value =
        serde_json::from_str(&std::fs::read_to_string(res.file_path).unwrap()).unwrap();
    let ids: Vec<String> = json["posts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(res.post_count, ids.len());
    ids
}

#[test]
fn export_pages_reach_posts_sharing_the_second_of_a_page_end() {
    use weflow_native::fixture::T0;
    // one more post than a page, all in the same second
    let times: Vec<(i64, i64)> = (0..51).map(|i| (3001 + i, T0)).collect();
    let mut ids = export_posts_with_times("sns-export-same-second", &times);
    assert_eq!(ids.len(), 51);
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 51, "every post once");
}

#[test]
fn export_ends_when_a_full_page_ends_with_posts_without_time() {
    use weflow_native::fixture::T0;
    // the first page is full and ends with posts whose time is 0
    let times: Vec<(i64, i64)> = std::iter::once((5000, T0))
        .chain((0..50).map(|i| (5001 + i, 0)))
        .collect();
    let mut ids = export_posts_with_times("sns-export-zero-time", &times);
    assert_eq!(ids.len(), 51);
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 51, "every post once");
}

/// The names in `dir`, sorted; none when it is not there.
fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// A hub over an account whose Moments are `posts` and whose contacts are `contacts`.
fn moments_hub(
    tag: &str,
    posts: &[weflow_native::fixture::SnsPostSpec],
    contacts: &[weflow_native::fixture::ContactSpec],
) -> (
    weflow_native::fixture::TestDir,
    weflow_core::services::ServiceHub,
    weflow_native::fixture::Fixture,
) {
    common::custom_hub(tag, |f| {
        f.session_db(&[]);
        f.contact_db(contacts, &[]);
        f.sns_db(posts);
    })
}

/// A text post of `wxid_bob` carrying `extra` XML.
fn bob_post(tid: i64, extra: &str) -> weflow_native::fixture::SnsPostSpec<'_> {
    weflow_native::fixture::SnsPostSpec {
        tid,
        user: "wxid_bob",
        create_time: weflow_native::fixture::T0,
        desc: "post",
        kind: 1,
        media: 0,
        likes: &[],
        extra,
    }
}

fn export_options(dir: &Path, format: &str) -> SnsExportOptions {
    SnsExportOptions {
        output_dir: dir.to_path_buf(),
        format: format.into(),
        ..Default::default()
    }
}

#[test]
fn export_reports_a_moments_database_that_cannot_be_read() {
    let (root, hub, f) = moments_hub("sns-export-unreadable", &[bob_post(9001, "")], &[]);
    // every page after the first is zeros: the database is malformed from the first read on (#1037)
    let db = f.db_storage().join("sns").join("sns.db");
    let mut bytes = std::fs::read(&db).unwrap();
    assert!(bytes.len() > 4096, "the database has pages after the first");
    bytes[4096..].fill(0);
    std::fs::write(&db, bytes).unwrap();
    let timeline = hub.sns_timeline_query(&SnsTimelineQuery {
        limit: 1,
        ..SnsTimelineQuery::default()
    });
    assert!(timeline.is_err(), "the timeline reports it");

    let out = root.join("out");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let err = rt
        .block_on(hub.sns_export_timeline(&export_options(&out, "json")))
        .unwrap_err();
    assert_eq!(err.code, "native_error", "{}", err.message);
    assert_eq!(names_in(&out), Vec::<String>::new(), "no export file");
}

#[test]
fn export_refuses_a_format_it_does_not_write_before_writing_anything() {
    let (root, hub) = common::mock_hub("sns-export-format");
    let rt = tokio::runtime::Runtime::new().unwrap();
    for (i, format) in ["bogus", "xlsx", "csv", "arkme-json", ""]
        .iter()
        .enumerate()
    {
        let out = root.join(format!("out-{i}"));
        let err = rt
            .block_on(hub.sns_export_timeline(&export_options(&out, format)))
            .unwrap_err();
        assert_eq!(err.code, "invalid_arguments", "{format:?}");
        assert!(
            err.message.contains("supported: json, html, arkmejson"),
            "{format:?}: {}",
            err.message
        );
        assert!(!out.exists(), "{format:?}: the folder is not made");
    }
}

#[test]
fn export_format_is_read_in_any_case_and_html_in_any_case_gets_its_avatars() {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend((0..300u32).map(|i| (i % 251) as u8));
    let base = serve(vec![("/avatar", vec![], png)]);
    let avatar = format!("{base}/avatar");
    let contacts = [weflow_native::fixture::ContactSpec {
        avatar: &avatar,
        ..weflow_native::fixture::ContactSpec::new("wxid_bob", 1, "Bob")
    }];
    let (root, hub, _f) = moments_hub("sns-export-case", &[bob_post(9101, "")], &contacts);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut files = Vec::new();
    for (i, (format, extension)) in [
        ("JSON", "json"),
        (" Html ", "html"),
        ("ArkmeJson", "json"),
        ("HTML", "html"),
    ]
    .into_iter()
    .enumerate()
    {
        let out = root.join(format!("out-{i}"));
        let res = rt
            .block_on(hub.sns_export_timeline(&export_options(&out, format)))
            .unwrap();
        assert_eq!(res.post_count, 1, "{format:?}");
        assert_eq!(
            res.file_path.extension().and_then(|e| e.to_str()),
            Some(extension),
            "{format:?}"
        );
        files.push((out, res.file_path));
    }
    // the file of each format is its own: the arkme export names its format
    let ark: Value = serde_json::from_str(&std::fs::read_to_string(&files[2].1).unwrap()).unwrap();
    assert_eq!(ark["format"], "arkmejson");
    let plain: Value =
        serde_json::from_str(&std::fs::read_to_string(&files[0].1).unwrap()).unwrap();
    assert_eq!(plain.get("format"), None);
    // a page in whatever case is written with the avatars its folder holds
    for i in [1, 3] {
        let media = names_in(&files[i].0.join("media"));
        assert!(
            matches!(media.as_slice(), [one] if one.starts_with("avatar_") && one.ends_with(".png")),
            "{media:?}"
        );
        let html = std::fs::read_to_string(&files[i].1).unwrap();
        assert!(html.contains(&media[0]), "the page shows the avatar");
    }
    assert!(
        !files[0].0.join("media").exists(),
        "a json export has no avatars"
    );
}

#[test]
fn export_saves_each_medium_under_the_extension_of_what_it_holds() {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend((0..300u32).map(|i| (i % 251) as u8));
    let mut gif = b"GIF89a".to_vec();
    gif.extend((0..300u32).map(|i| (i % 241) as u8));
    let mut jpeg = fake_jpeg();
    jpeg.extend((0..300u32).map(|i| (i % 239) as u8));
    let mut video = vec![0, 0, 0, 0x18];
    video.extend_from_slice(b"ftypmp42");
    video.extend((0..400u32).map(|i| (i % 200) as u8));
    let base = serve(vec![
        ("/pic", vec![], png),
        ("/anim", vec![], gif),
        ("/photo", vec![], jpeg),
        ("/snsvideodownload/clip", vec![], video),
    ]);
    // the addresses of a post's media are made https; the scheme in capitals is left as it is
    let base = base.replacen("http", "HTTP", 1);
    let media: String = ["pic", "anim", "photo", "snsvideodownload/clip"]
        .iter()
        .map(|name| format!("<media><url>{base}/{name}</url></media>"))
        .collect();
    let (root, hub, _f) = moments_hub("sns-export-media", &[bob_post(9201, &media)], &[]);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let out = root.join("out");
    let opts = SnsExportOptions {
        export_media: true,
        ..export_options(&out, "json")
    };
    let res = rt.block_on(hub.sns_export_timeline(&opts)).unwrap();
    assert_eq!(res.media_count, 4);
    let exported: Value =
        serde_json::from_str(&std::fs::read_to_string(&res.file_path).unwrap()).unwrap();
    let local: Vec<&str> = exported["posts"][0]["media"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["localPath"].as_str().unwrap())
        .collect();
    let want = [
        "media/9201_0.png",
        "media/9201_1.gif",
        "media/9201_2.jpg",
        "media/9201_3.mp4",
    ];
    assert_eq!(local, want);
    let names = names_in(&out.join("media"));
    assert_eq!(
        names,
        ["9201_0.png", "9201_1.gif", "9201_2.jpg", "9201_3.mp4"]
    );

    // the next export into the same folder finds them, and saves none a second time
    let again = rt.block_on(hub.sns_export_timeline(&opts)).unwrap();
    assert_eq!(again.media_count, 4);
    assert_eq!(names_in(&out.join("media")), names);
}

#[test]
fn export_file_name_carries_the_local_time_and_the_data_the_utc_time() {
    let (root, hub) = common::mock_hub("sns-export-stamp");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let res = rt
        .block_on(hub.sns_export_timeline(&export_options(&root.join("out"), "json")))
        .unwrap();
    let name = res.file_path.file_stem().unwrap().to_string_lossy();
    let stamp = name.strip_prefix("朋友圈导出_").unwrap();
    let named = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%dT%H-%M-%S").unwrap();
    let now = chrono::Local::now().naive_local();
    assert!(
        (now - named).num_seconds().abs() < 120,
        "{stamp} is not the local time {now}"
    );
    let exported: Value =
        serde_json::from_str(&std::fs::read_to_string(&res.file_path).unwrap()).unwrap();
    let time = exported["exportTime"].as_str().unwrap();
    assert!(time.ends_with('Z'), "{time} is UTC");
    let utc = chrono::DateTime::parse_from_rfc3339(time).unwrap();
    assert!((chrono::Utc::now() - utc.to_utc()).num_seconds().abs() < 120);
}

#[test]
fn debug_resource_reports_status_and_decryption_headers() {
    let (_root, hub) = common::mock_hub("sns-debug");
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
    let r = serde_json::to_value(
        rt.block_on(hub.sns_debug_resource(&format!("{base}/res")))
            .unwrap(),
    )
    .unwrap();
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
    let missing = rt
        .block_on(hub.sns_debug_resource(&format!("{base}/missing")))
        .unwrap();
    assert_eq!(missing.status, 404);
    assert!(rt.block_on(hub.sns_debug_resource(" ")).is_err());
    assert!(
        rt.block_on(hub.sns_debug_resource("http://127.0.0.1:9/x"))
            .is_err(),
        "connection refused"
    );
}
