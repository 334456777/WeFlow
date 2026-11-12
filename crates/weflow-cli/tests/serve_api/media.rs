//! Media: images and voices as files, videos in parts, Moments media.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::common::{
    account_home, account_home_with, form_parts, percent, run_cli, start, temp_root, TOKEN,
};

#[test]
fn media_come_back_as_files_from_the_process() {
    use weflow_native::fixture::{VoiceSpec, DAY, T0};
    let root = temp_root("media");
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
    jpeg.extend((0..3000u32).map(|i| (i as u8).wrapping_mul(7) | 1));
    let voice_at = T0 + 500;
    let mut img_dir = PathBuf::new();
    let home = account_home_with(&root, |f| {
        // the image of `wxid_bob`'s message 3 (an old-style .dat: the whole file XOR-ed) and a real SILK voice
        let img = f
            .account_dir
            .join("msg/attach")
            .join(weflow_core::image::session_dir_for_storage("wxid_bob"))
            .join(weflow_core::image::year_month_from_create_time(Some(
                T0 + DAY,
            )))
            .join("Img");
        std::fs::create_dir_all(&img).unwrap();
        std::fs::write(
            img.join("aabbccddeeff00112233445566778899.dat"),
            jpeg.iter().map(|b| b ^ 0x5a).collect::<Vec<u8>>(),
        )
        .unwrap();
        img_dir = img;
        f.media_db(
            0,
            &[VoiceSpec {
                chat: "wxid_bob",
                create_time: voice_at,
                local_id: 6,
                svr_id: 9_000_000_000_001,
                data: weflow_silk::testenc::encode_tone(24000),
                index: "0",
            }],
        );
    });
    let (exit, _, stderr) = run_cli(&home, &["config", "set", "image_xor_key", "90"]);
    assert_eq!(exit, 0, "{stderr}");
    let server = start(
        &home,
        &["--api-token", TOKEN, "--max-media", "1", "--max-reads", "1"],
    );
    let auth = format!("Bearer {TOKEN}");

    // the image: its bytes, the same as the child process writes, with an ETag that answers 304
    let query = format!("session-id=wxid_bob&msg-id=3&create-time={}", T0 + DAY);
    let r = server.call("GET", &format!("/api/chat/image-data?{query}"), "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.headers["content-type"], "image/jpeg");
    assert_eq!(r.bytes, jpeg);
    assert!(r.headers["cache-control"].contains("max-age"));
    // the time in line for a place to decode, then the decoding
    let timing = &r.headers["server-timing"];
    assert!(
        timing.starts_with("wait;") && timing.contains(", cmd;desc=\"in process\";"),
        "{timing}"
    );
    let etag = r.headers["etag"].clone();
    let child = server.call(
        "POST",
        "/chat/image-data?return=file",
        &format!(
            r#"{{"session-id": "wxid_bob", "msg-id": "3", "create-time": {}}}"#,
            T0 + DAY
        ),
    );
    assert_eq!(child.status, 200, "{}", child.body);
    assert_eq!(form_parts(&child)[1].1, jpeg);
    let again = server.request(
        "GET",
        &format!("/api/chat/image-data?{query}"),
        &[("Authorization", &auth), ("If-None-Match", &etag)],
        "",
    );
    assert_eq!(again.status, 304);
    assert!(again.bytes.is_empty());
    assert_eq!(again.headers["etag"], etag);
    let other = server.request(
        "GET",
        &format!("/api/chat/image-data?{query}"),
        &[("Authorization", &auth), ("If-None-Match", "\"0\"")],
        "",
    );
    assert_eq!(other.status, 200);
    // the file as it is, even to a client that takes compressed answers: an image is compressed already
    let plain = server.request(
        "GET",
        &format!("/api/chat/image-data?{query}"),
        &[("Authorization", &auth), ("Accept-Encoding", "gzip, br")],
        "",
    );
    assert_eq!(plain.headers.get("content-encoding"), None);
    assert_eq!(plain.bytes, jpeg);

    // a part of it, as a player or a resumed download asks: 206, or 416 past the end, or the whole file when it
    // changed since (`If-Range`)
    let part = |range: &str, if_range: &str| {
        let mut headers = vec![
            ("Authorization", auth.as_str()),
            ("Range", range),
            ("Accept-Encoding", "gzip"),
        ];
        if !if_range.is_empty() {
            headers.push(("If-Range", if_range));
        }
        server.request(
            "GET",
            &format!("/api/chat/image-data?{query}"),
            &headers,
            "",
        )
    };
    let len = jpeg.len();
    let r = part("bytes=0-9", "");
    assert_eq!(r.status, 206);
    assert_eq!(r.bytes, jpeg[..10]);
    assert_eq!(r.headers["content-range"], format!("bytes 0-9/{len}"));
    assert_eq!(r.headers["accept-ranges"], "bytes");
    assert_eq!(r.headers.get("content-encoding"), None);
    let r = part("bytes=-4", &etag);
    assert_eq!((r.status, r.bytes.as_slice()), (206, &jpeg[len - 4..]));
    let r = part("bytes=0-9", "\"0\"");
    assert_eq!((r.status, r.bytes.len()), (200, len));
    let r = part(&format!("bytes={len}-"), "");
    assert_eq!(r.status, 416);
    assert_eq!(r.headers["content-range"], format!("bytes */{len}"));

    // a thumbnail for a list: WeChat's own `_t.dat` when there is one, else the image
    let thumb = format!("/api/chat/image-data?{query}&thumb=true");
    assert_eq!(server.call("GET", &thumb, "").bytes, jpeg);
    let small: Vec<u8> = [0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3].to_vec();
    std::fs::write(
        img_dir.join("aabbccddeeff00112233445566778899_t.dat"),
        small.iter().map(|b| b ^ 0x5a).collect::<Vec<u8>>(),
    )
    .unwrap();
    let r = server.call("GET", &thumb, "");
    assert_eq!(
        (r.status, r.headers["content-type"].as_str()),
        (200, "image/jpeg")
    );
    assert_eq!(r.bytes, small);
    let child = server.call(
        "POST",
        "/chat/image-data?return=file",
        &format!(
            r#"{{"session-id": "wxid_bob", "msg-id": "3", "create-time": {}, "thumb": true}}"#,
            T0 + DAY
        ),
    );
    assert_eq!(form_parts(&child)[1].1, small);
    // and the image itself is still the image
    assert_eq!(
        server
            .call("GET", &format!("/api/chat/image-data?{query}"), "")
            .bytes,
        jpeg
    );

    // the files WeChat keeps of it (#194): the thumbnail and the image are here, the HD original is not yet
    let info = format!("/api/chat/image-info?{query}");
    let r = server.call("GET", &info, "");
    assert_eq!(r.status, 200, "{}", r.body);
    let v = r.json()["data"].clone();
    assert_eq!(
        (v["thumb"].clone(), v["normal"].clone(), v["hd"].clone()),
        (json!(true), json!(true), json!(false))
    );
    assert_eq!(
        server.call("GET", &info, "").body,
        server
            .call("GET", &format!("/chat/image-info?{query}"), "")
            .body
    );
    let hd_url = format!("/api/chat/image-data?{query}&variant=hd");
    assert_eq!(server.call("GET", &hd_url, "").status, 404);
    // asking WeChat to download it needs the hook, which this server does not run (#766)
    let requested = server.call(
        "POST",
        "/api/image/download-original",
        &format!(
            r#"{{"session-id": "wxid_bob", "msg-id": "3", "create-time": {}}}"#,
            T0 + DAY
        ),
    );
    assert_eq!(requested.status, 501, "{}", requested.body);
    assert_eq!(requested.json()["error"]["code"], "unsupported");
    let normal_url = format!("/api/chat/image-data?{query}&variant=normal");
    assert_eq!(server.call("GET", &normal_url, "").bytes, jpeg);
    // once WeChat has downloaded it: `hd` is the original, and `normal` still the image opened
    let mut original = vec![0xFF, 0xD8, 0xFF, 0xE0];
    original.extend((0..6000u32).map(|i| (i as u8).wrapping_mul(13) | 1));
    std::fs::write(
        img_dir.join("aabbccddeeff00112233445566778899_h.dat"),
        original.iter().map(|b| b ^ 0x5a).collect::<Vec<u8>>(),
    )
    .unwrap();
    assert_eq!(server.call("GET", &info, "").json()["data"]["hd"], true);
    let r = server.call("GET", &hd_url, "");
    assert_eq!((r.status, r.bytes == original), (200, true));
    assert_eq!(server.call("GET", &normal_url, "").bytes, jpeg);
    assert_eq!(
        server
            .call("GET", &format!("{normal_url}&thumb=true"), "")
            .status,
        400
    );
    std::fs::remove_file(img_dir.join("aabbccddeeff00112233445566778899_h.dat")).unwrap();

    // a screen of images asked for at once, one decoded at a time: they wait in line instead of answering 429, and
    // take no place of the other reads
    let statuses: Vec<u16> = std::thread::scope(|s| {
        let all: Vec<_> = (0..20)
            .map(|_| s.spawn(|| server.call("GET", &format!("/api/chat/image-data?{query}"), "")))
            .collect();
        assert_eq!(server.call("GET", "/api/chat/sessions", "").status, 200);
        all.into_iter().map(|h| h.join().unwrap().status).collect()
    });
    assert!(statuses.iter().all(|&s| s == 200), "{statuses:?}");
    // so do the reads of /api/, one at a time here; a child process route is refused at once instead
    let at_once = |target: &str, n: usize| -> Vec<u16> {
        std::thread::scope(|s| {
            let all: Vec<_> = (0..n)
                .map(|_| s.spawn(|| server.call("GET", target, "").status))
                .collect();
            all.into_iter().map(|h| h.join().unwrap()).collect()
        })
    };
    let statuses = at_once("/api/chat/sessions?limit=1", 12);
    assert!(statuses.iter().all(|&s| s == 200), "{statuses:?}");
    let statuses = at_once("/chat/sessions?limit=1", 8);
    assert!(statuses.contains(&429), "{statuses:?}");

    // the voice, decoded to WAV: the same as the child process prints in base64
    let query = format!(
        "session-id=wxid_bob&msg-id=6&create-time={voice_at}&server-id=9000000000001&sender=wxid_bob"
    );
    let r = server.call("GET", &format!("/api/chat/voice-data?{query}"), "");
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.headers["content-type"], "audio/wav");
    assert_eq!(&r.bytes[..4], b"RIFF");
    let wav = server.request(
        "GET",
        &format!("/api/chat/voice-data?{query}"),
        &[("Authorization", &auth), ("Accept-Encoding", "gzip")],
        "",
    );
    assert_eq!(wav.headers.get("content-encoding"), None);
    assert_eq!(wav.bytes, r.bytes);
    let child = server.call(
        "POST",
        "/chat/voice-data",
        &format!(
            r#"{{"session-id": "wxid_bob", "msg-id": "6", "create-time": {voice_at}, "server-id": "9000000000001", "sender": "wxid_bob"}}"#
        ),
    );
    assert_eq!(child.status, 200, "{}", child.body);
    let printed = child.json()["data"]
        .as_object()
        .unwrap()
        .values()
        .find_map(|v| v.as_str().filter(|s| s.len() > 100).map(String::from))
        .expect("the WAV in base64");
    use base64::Engine;
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(printed)
            .unwrap(),
        r.bytes
    );

    // failures are the command's --json document, as from the child process
    let failed = |api: &str, child_body: &str| {
        let own = server.call("GET", api, "");
        let child = server.call("POST", &api["/api".len()..], child_body);
        assert_ne!(own.status, 200, "{api}");
        assert_eq!(
            (own.status, own.json()["error"]["code"].clone()),
            (child.status, child.json()["error"]["code"].clone()),
            "{api}: {} / {}",
            own.body,
            child.body
        );
    };
    // a text message has no image
    failed(
        "/api/chat/image-data?session-id=wxid_bob&msg-id=1",
        r#"{"session-id": "wxid_bob", "msg-id": "1", "out": "unused.jpg"}"#,
    );
    failed(
        "/api/chat/voice-data?session-id=wxid_bob&msg-id=99",
        r#"{"session-id": "wxid_bob", "msg-id": "99"}"#,
    );
    // a command line that does not parse is reported by the child process, which writes nothing
    let r = server.call("GET", "/api/chat/image-data?session-id=wxid_bob", "");
    assert_eq!(r.status, 400, "{}", r.body);
    assert_eq!(r.json()["error"]["code"], "invalid_arguments");
    assert!(!Path::new("-").exists());

    // only GET, and the file is the answer
    let r = server.call("POST", &format!("/api/chat/image-data?{query}"), "");
    assert_eq!(r.status, 405);
    assert_eq!(r.headers["allow"], "GET");
    let r = server.call(
        "GET",
        "/api/chat/image-data?session-id=wxid_bob&msg-id=3&out=x.jpg",
        "",
    );
    assert_eq!(r.status, 400, "{}", r.body);
}

/// An open `GET /api/push/messages` and what it sent so far.
#[test]
fn videos_are_sent_in_the_parts_asked_for() {
    use weflow_native::fixture::{MsgSpec, DAY, T0};
    let root = temp_root("video");
    let md5 = "0123456789abcdef0123456789abcdef";
    let at = T0 + 4 * DAY + 60;
    // larger than one chunk read, so a range crosses chunks
    let video: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    let (cover, thumb) = (b"\xFF\xD8\xFFcover".to_vec(), b"\xFF\xD8\xFFthumb".to_vec());
    let mut folder = PathBuf::new();
    let home = account_home_with(&root, |f| {
        let xml = |md5: &str| {
            format!("<msg><videomsg md5=\"{md5}\" length=\"700000\" playlength=\"3\" /></msg>")
        };
        f.message_shard(
            1,
            &[(
                "wxid_bob",
                vec![
                    MsgSpec::text(4, "wxid_me", T0 + 3 * DAY, "later"),
                    MsgSpec::text(5, "wxid_bob", T0 + 4 * DAY, "see you"),
                    MsgSpec::text(7, "wxid_bob", at, &xml(md5)).of_type(43),
                    // WeChat has not downloaded this one
                    MsgSpec::text(
                        8,
                        "wxid_bob",
                        at + 60,
                        &xml("ffffffffffffffffffffffffffffffff"),
                    )
                    .of_type(43),
                ],
            )],
        );
        folder = f
            .account_dir
            .join("msg/video")
            .join(weflow_core::image::year_month_from_create_time(Some(at)));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("{md5}.mp4")), &video).unwrap();
        std::fs::write(folder.join(format!("{md5}.jpg")), &cover).unwrap();
        std::fs::write(folder.join(format!("{md5}_thumb.jpg")), &thumb).unwrap();
    });
    let server = start(&home, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");
    let query = format!("session-id=wxid_bob&msg-id=7&create-time={at}");
    let get = |extra: &[(&str, &str)], q: &str| {
        let mut headers = vec![("Authorization", auth.as_str())];
        headers.extend_from_slice(extra);
        server.request("GET", &format!("/api/chat/video-data?{q}"), &headers, "")
    };

    // the whole video, then parts of it, as a player asks while it plays and seeks
    let r = get(&[], &query);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.headers["content-type"], "video/mp4");
    assert_eq!(r.headers["content-length"], video.len().to_string());
    assert_eq!(r.headers["accept-ranges"], "bytes");
    assert_eq!(r.bytes, video);
    let etag = r.headers["etag"].clone();
    let len = video.len();
    for (range, first, last) in [
        ("bytes=0-1", 0, 1),
        ("bytes=200000-600000", 200_000, 600_000),
        ("bytes=650000-", 650_000, len - 1),
        ("bytes=-10", len - 10, len - 1),
    ] {
        let r = get(&[("Range", range)], &query);
        assert_eq!(r.status, 206, "{range}");
        assert_eq!(
            r.headers["content-range"],
            format!("bytes {first}-{last}/{len}")
        );
        assert_eq!(r.headers["content-length"], (last + 1 - first).to_string());
        assert_eq!(r.bytes, video[first..=last], "{range}");
    }
    assert_eq!(
        get(&[("Range", &format!("bytes={len}-"))], &query).status,
        416
    );
    assert_eq!(get(&[("If-None-Match", &etag)], &query).status, 304);
    // a changed file is another file
    let r = get(&[("Range", "bytes=0-1"), ("If-Range", &etag)], &query);
    assert_eq!(r.status, 206);
    std::fs::write(folder.join(format!("{md5}.mp4")), &video[..1000]).unwrap();
    let r = get(&[("Range", "bytes=0-1"), ("If-Range", &etag)], &query);
    assert_eq!((r.status, r.bytes.len()), (200, 1000));
    std::fs::write(folder.join(format!("{md5}.mp4")), &video).unwrap();

    // its thumbnail, else its cover
    let r = get(&[], &format!("{query}&thumb=true"));
    assert_eq!(
        (r.status, r.headers["content-type"].as_str()),
        (200, "image/jpeg")
    );
    assert_eq!(r.bytes, thumb);
    std::fs::remove_file(folder.join(format!("{md5}_thumb.jpg"))).unwrap();
    assert_eq!(get(&[], &format!("{query}&thumb=true")).bytes, cover);

    // through the child process: the video written to a file
    let child = server.call(
        "POST",
        "/chat/video-data?return=file",
        &format!(r#"{{"session-id": "wxid_bob", "msg-id": "7", "create-time": {at}}}"#),
    );
    assert_eq!(child.status, 200, "{}", child.body);
    let parts = form_parts(&child);
    assert_eq!(parts[1].1, video);
    let result: Value = serde_json::from_slice(&parts[0].1).unwrap();
    assert_eq!(result["success"], true, "{result}");

    // not on this computer: 404; not a video: 400
    let r = get(
        &[],
        &format!("session-id=wxid_bob&msg-id=8&create-time={}", at + 60),
    );
    assert_eq!(r.status, 404, "{}", r.body);
    assert_eq!(r.json()["error"]["details"]["failureKind"], "not_found");
    let r = get(
        &[],
        &format!("session-id=wxid_bob&msg-id=5&create-time={}", T0 + 4 * DAY),
    );
    assert_eq!(r.status, 400, "{}", r.body);
}

#[test]
fn moments_media_come_from_the_cache_and_only_from_wechat() {
    use md5::{Digest, Md5};
    // what `sns media` fetched before is kept in the cache, named after the md5 of its address: the route reads it
    // from there, without the network (#202)
    let root = temp_root("sns-media");
    let home = account_home(&root);
    let (exit, listed, _) = run_cli(&home, &["--json", "cache", "list"]);
    assert_eq!(exit, 0);
    let listed: Value = serde_json::from_str(&listed).unwrap();
    let folder = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|part| part["part"] == "sns")
        .and_then(|part| part["paths"][0].as_str())
        .map(PathBuf::from)
        .expect("the Moments cache folder");
    std::fs::create_dir_all(&folder).unwrap();
    let cached = |url: &str, ext: &str, bytes: &[u8]| {
        let name = weflow_native::db::hex(&Md5::digest(url.as_bytes())) + ext;
        std::fs::write(folder.join(name), bytes).unwrap();
    };
    let image_url = "https://shmmsns.qpic.cn/mmsns/abc/0?token=T&idx=1";
    let image = b"\xFF\xD8\xFF\xE0moments".to_vec();
    cached(image_url, ".jpg", &image);
    let video_url = "https://szzjwxsns.video.qq.com/102/20202/snsvideodownload?m=x&token=T&idx=1";
    let video: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
    cached(video_url, ".mp4", &video);

    let server = start(&home, &["--api-token", TOKEN]);
    let auth = format!("Bearer {TOKEN}");
    let get = |url: &str, extra: &[(&str, &str)]| {
        let mut headers = vec![("Authorization", auth.as_str())];
        headers.extend_from_slice(extra);
        let q = format!("url={}&key=123", percent(url));
        server.request("GET", &format!("/api/sns/media?{q}"), &headers, "")
    };

    let r = get(image_url, &[]);
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.headers["content-type"], "image/jpeg");
    assert_eq!(r.bytes, image);
    assert_eq!(
        get(image_url, &[("If-None-Match", &r.headers["etag"])]).status,
        304
    );
    // a video is sent from its file, in the parts a player asks for
    let r = get(video_url, &[("Range", "bytes=1000-1999")]);
    assert_eq!(r.status, 206, "{}", r.body);
    assert_eq!(r.headers["content-type"], "video/mp4");
    assert_eq!(
        r.headers["content-range"],
        format!("bytes 1000-1999/{}", video.len())
    );
    assert_eq!(r.bytes, video[1000..2000]);

    // only https addresses of WeChat's media servers: the route does not fetch whatever it is given
    for url in [
        "http://shmmsns.qpic.cn/mmsns/abc/0",
        "https://127.0.0.1/secret",
        "https://qpic.cn.example.com/a",
        "file:///etc/hosts",
    ] {
        let r = get(url, &[]);
        assert_eq!(r.status, 400, "{url}: {}", r.body);
        assert_eq!(r.json()["error"]["code"], "invalid_arguments", "{url}");
    }
    // and it needs the token, as every /api/ route does
    assert_eq!(
        server
            .request(
                "GET",
                "/api/sns/media?url=https%3A%2F%2Fa.qpic.cn%2Fx",
                &[],
                ""
            )
            .status,
        401
    );
}

/// The images of a conversation whose username ends like a folder suffix (`name_1998`) are looked for under the
/// md5 of the whole username first, then under the md5 of the name without the suffix, where the desktop app
/// looked (#657): both are found, by the server's own reads and by the command it runs.
#[test]
fn images_of_a_username_with_a_folder_like_suffix_are_found() {
    use md5::{Digest, Md5};
    use weflow_native::fixture::{MsgSpec, DAY, T0};
    let md5_hex = |s: &str| weflow_native::db::hex(&Md5::digest(s.as_bytes()));
    let root = temp_root("suffixed-session");
    let at = T0 + 6 * DAY;
    // (session, the folder its images are under, the image's md5, the image)
    let cases: Vec<(&'static str, &str, String, Vec<u8>)> =
        [("name_1998", "name_1998"), ("name_2001", "name")]
            .into_iter()
            .enumerate()
            .map(|(i, (session, folder))| {
                let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
                jpeg.extend((0..2000u32).map(|n| (n as u8).wrapping_mul(i as u8 + 3) | 1));
                (session, folder, md5_hex(session), jpeg)
            })
            .collect();
    let home = account_home_with(&root, |f| {
        let sessions: Vec<(&str, Vec<MsgSpec>)> = cases
            .iter()
            .map(|(session, _, image_md5, _)| {
                let xml = format!("<msg><img md5=\"{image_md5}\"/></msg>");
                (
                    *session,
                    vec![MsgSpec::text(1, session, at, &xml).of_type(3)],
                )
            })
            .collect();
        f.message_shard(2, &sessions);
        for (_, folder, image_md5, jpeg) in &cases {
            // an old-style .dat: the whole file XOR-ed
            let img = f
                .account_dir
                .join("msg/attach")
                .join(md5_hex(folder))
                .join(weflow_core::image::year_month_from_create_time(Some(at)))
                .join("Img");
            std::fs::create_dir_all(&img).unwrap();
            std::fs::write(
                img.join(format!("{image_md5}.dat")),
                jpeg.iter().map(|b| b ^ 0x5a).collect::<Vec<u8>>(),
            )
            .unwrap();
        }
    });
    let (exit, _, stderr) = run_cli(&home, &["config", "set", "image_xor_key", "90"]);
    assert_eq!(exit, 0, "{stderr}");
    let server = start(&home, &["--api-token", TOKEN]);
    for (session, _, _, jpeg) in &cases {
        let own = server.call(
            "GET",
            &format!("/api/chat/image-data?session-id={session}&msg-id=1&create-time={at}"),
            "",
        );
        assert_eq!(own.status, 200, "{session}: {}", own.body);
        assert_eq!(&own.bytes, jpeg, "{session}");
        let child = server.call(
            "POST",
            "/chat/image-data?return=file",
            &json!({ "session-id": session, "msg-id": "1", "create-time": at }).to_string(),
        );
        assert_eq!(child.status, 200, "{session}: {}", child.body);
        assert_eq!(&form_parts(&child)[1].1, jpeg, "{session}");
    }
}
