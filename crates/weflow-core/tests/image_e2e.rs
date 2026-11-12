#![expect(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![expect(
    clippy::cast_possible_truncation,
    reason = "test values are known to fit, or are meant to wrap"
)]

mod common;

use aes::cipher::{Array, BlockModeEncrypt, KeyInit};
use md5::{Digest, Md5};
use weflow_core::services::ImagePayload;
use weflow_native::fixture::TestDir;

const MD5: &str = "0123456789abcdef0123456789abcdef";
const KEY: &str = "0123456789abcdef";

fn jpeg(len: usize, seed: u8) -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
    v.extend((0..len - 4).map(|i| ((i as u8).wrapping_mul(7)).wrapping_add(seed) | 1));
    v
}

/// V2 `.dat`: 1 KiB AES-ECB head (PKCS7), raw middle, XOR-ed tail.
fn encrypt_v2(plain: &[u8], key: &[u8; 16], xor: u8) -> Vec<u8> {
    let aes_len = 1024.min(plain.len());
    let xor_len = 100;
    let mut padded = plain[..aes_len].to_vec();
    let pad = 16 - padded.len() % 16;
    padded.extend(std::iter::repeat_n(pad as u8, pad));
    let mut enc = ecb::Encryptor::<aes::Aes128>::new(key.into());
    let mut out = vec![0x07, 0x08, 0x56, 0x32, 0x08, 0x07];
    out.extend_from_slice(&(aes_len as i32).to_le_bytes());
    out.extend_from_slice(&(xor_len as i32).to_le_bytes());
    out.push(1);
    for chunk in padded.chunks(16) {
        let mut b = Array::try_from(chunk).unwrap();
        enc.encrypt_block(&mut b);
        out.extend_from_slice(&b);
    }
    out.extend_from_slice(&plain[aes_len..plain.len() - xor_len]);
    out.extend(plain[plain.len() - xor_len..].iter().map(|b| b ^ xor));
    out
}

/// A folder opened so that its times can be set (Windows opens a folder only with backup semantics, and setting times
/// needs the write-attributes right).
fn open_dir_to_set_times(dir: &std::path::Path) -> std::fs::File {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::File::options()
            .access_mode(0x0100)
            .custom_flags(0x0200_0000)
            .open(dir)
            .unwrap()
    }
    #[cfg(not(windows))]
    {
        std::fs::File::open(dir).unwrap()
    }
}

fn md5_hex(s: &str) -> String {
    Md5::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn setup(
    tag: &str,
) -> (
    TestDir,
    weflow_core::services::ServiceHub,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let (root, hub) = common::mock_hub_with(tag, |p| {
        p.image_xor_key = Some(0x5a);
        p.image_aes_key = Some(KEY.into());
    });
    let account = root.join("data/wxid_me_ab12");
    let month = weflow_core::image::year_month_from_create_time(Some(1_700_000_000));
    let img_dir = account
        .join("msg/attach")
        .join(md5_hex("wxid_bob"))
        .join(&month)
        .join("Img");
    std::fs::create_dir_all(&img_dir).unwrap();
    (root, hub, account, img_dir)
}

#[test]
fn decrypts_the_hd_variant_and_caches_it() {
    let (_root, hub, _account, img_dir) = setup("img-hd");
    let hd = jpeg(3000, 1);
    let thumb = jpeg(1500, 2);
    std::fs::write(
        img_dir.join(format!("{MD5}_h.dat")),
        encrypt_v2(&hd, KEY.as_bytes().try_into().unwrap(), 0x5a),
    )
    .unwrap();
    std::fs::write(
        img_dir.join(format!("{MD5}_t.dat")),
        encrypt_v2(&thumb, KEY.as_bytes().try_into().unwrap(), 0x5a),
    )
    .unwrap();

    let p = ImagePayload {
        session_id: Some("wxid_bob".into()),
        image_md5: Some(MD5.into()),
        create_time: Some(1_700_000_000),
        prefer_file_path: true,
        ..Default::default()
    };
    assert_eq!(
        hub.image_resolve_cache(&p).failure_kind,
        Some("not_found"),
        "nothing cached yet"
    );

    let r = hub.image_decrypt(&p);
    assert!(r.success, "{:?}", r.error);
    let path = r.local_path.unwrap();
    assert!(path.ends_with(&format!("{MD5}_hd.jpg")), "{path}");
    assert!(
        path.replace('\\', "/").contains("Images/wxid_bob/"),
        "{path}"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        hd,
        "HD rendition wins over the thumbnail"
    );
    assert_eq!(r.is_thumb, Some(false));

    // resolving from the cache now succeeds without touching the .dat files
    let again = hub.image_resolve_cache(&p);
    assert!(again.success);
    assert_eq!(again.local_path.as_deref(), Some(path.as_str()));
    assert_eq!(again.has_update, Some(false));

    // without preferFilePath the result is a data URL
    let data = hub.image_resolve_cache(&ImagePayload {
        prefer_file_path: false,
        ..p.clone()
    });
    assert!(data
        .local_path
        .unwrap()
        .starts_with("data:image/jpeg;base64,"));

    // batch: one row per payload, duplicates answered identically
    let batch = serde_json::to_value(hub.image_resolve_cache_batch(&[
        p.clone(),
        p,
        ImagePayload {
            image_md5: Some("f".repeat(32)),
            ..Default::default()
        },
    ]))
    .unwrap();
    assert_eq!(batch["rows"].as_array().unwrap().len(), 3);
    assert_eq!(batch["rows"][0], batch["rows"][1]);
    assert_eq!(batch["rows"][2]["success"], false);

    let cleared = hub.cache_clear(&[weflow_core::services::CachePart::Images]);
    assert!(cleared.warnings.is_empty(), "{:?}", cleared.warnings);
    assert!(!std::path::Path::new(&path).exists());
}

#[test]
fn a_username_with_a_folder_like_suffix_is_looked_for_whole_then_cut() {
    let (_root, hub, account, _img_dir) = setup("img-suffix");
    let key: [u8; 16] = KEY.as_bytes().try_into().unwrap();
    let month = weflow_core::image::year_month_from_create_time(Some(1_700_000_000));
    // `name_1998` keeps its images under md5 of the whole username; `name_2001`'s are under md5("name"), where the
    // desktop app looked (#657)
    for (session, folder, seed) in [("name_1998", "name_1998", 4), ("name_2001", "name", 5)] {
        let dir = account
            .join("msg/attach")
            .join(md5_hex(folder))
            .join(&month)
            .join("Img");
        std::fs::create_dir_all(&dir).unwrap();
        // an image md5 of its own, so that neither finds the other's in the cache
        let image_md5 = md5_hex(session);
        let image = jpeg(2000, seed);
        std::fs::write(
            dir.join(format!("{image_md5}_t.dat")),
            encrypt_v2(&image, &key, 0x5a),
        )
        .unwrap();
        let r = hub.image_decrypt(&ImagePayload {
            session_id: Some(session.into()),
            image_md5: Some(image_md5),
            create_time: Some(1_700_000_000),
            prefer_file_path: true,
            allow_cache_index: Some(false),
            ..Default::default()
        });
        assert!(r.success, "{session}: {:?}", r.error);
        assert_eq!(
            std::fs::read(r.local_path.unwrap()).unwrap(),
            image,
            "{session}"
        );
    }
}

#[test]
fn falls_back_to_the_thumbnail_and_promotes_it_when_hd_appears() {
    let (_root, hub, _account, img_dir) = setup("img-thumb");
    let thumb = jpeg(1500, 3);
    let key: [u8; 16] = KEY.as_bytes().try_into().unwrap();
    std::fs::write(
        img_dir.join(format!("{MD5}_t.dat")),
        encrypt_v2(&thumb, &key, 0x5a),
    )
    .unwrap();

    let p = ImagePayload {
        session_id: Some("wxid_bob".into()),
        image_md5: Some(MD5.into()),
        create_time: Some(1_700_000_000),
        prefer_file_path: true,
        ..Default::default()
    };
    let r = hub.image_decrypt(&p);
    assert!(r.success, "{:?}", r.error);
    let path = r.local_path.unwrap();
    assert!(path.ends_with(&format!("{MD5}_t.jpg")), "{path}");
    assert_eq!(r.is_thumb, Some(true));

    // force with no HD file still succeeds, falling back to the thumbnail
    let forced = hub.image_decrypt(&ImagePayload {
        force: true,
        ..p.clone()
    });
    assert!(forced.success);

    // the HD file shows up later: resolving the cache upgrades the thumbnail in place
    let hd = jpeg(3000, 4);
    std::fs::write(
        img_dir.join(format!("{MD5}_h.dat")),
        encrypt_v2(&hd, &key, 0x5a),
    )
    .unwrap();
    let upgraded = hub.image_resolve_cache(&p);
    assert!(upgraded.success, "{:?}", upgraded.error);
    let new_path = upgraded.local_path.unwrap();
    assert!(new_path.ends_with(&format!("{MD5}_hd.jpg")), "{new_path}");
    assert_eq!(std::fs::read(&new_path).unwrap(), hd);
    assert!(
        !std::path::Path::new(&path).exists(),
        "the thumbnail cache file is removed after the upgrade"
    );
}

#[test]
fn the_cache_sees_files_changed_by_another_process() {
    let (_root, hub, _account, img_dir) = setup("img-cache-names");
    let key: [u8; 16] = KEY.as_bytes().try_into().unwrap();
    let other = "fedcba9876543210fedcba9876543210";
    let (mine, theirs) = (jpeg(3000, 6), jpeg(3000, 7));
    std::fs::write(
        img_dir.join(format!("{MD5}_h.dat")),
        encrypt_v2(&mine, &key, 0x5a),
    )
    .unwrap();
    std::fs::write(
        img_dir.join(format!("{other}_h.dat")),
        encrypt_v2(&jpeg(3000, 8), &key, 0x5a),
    )
    .unwrap();
    let payload = |md5: &str| ImagePayload {
        session_id: Some("wxid_bob".into()),
        image_md5: Some(md5.into()),
        create_time: Some(1_700_000_000),
        prefer_file_path: true,
        ..Default::default()
    };
    let path = std::path::PathBuf::from(hub.image_decrypt(&payload(MD5)).local_path.unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), mine);
    let cache_dir = path.parent().unwrap();

    // another process caches the second image: the folder's time changes and its names are read again
    std::fs::write(cache_dir.join(format!("{other}_hd.jpg")), &theirs).unwrap();
    open_dir_to_set_times(cache_dir)
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
        .unwrap();
    let found = hub.image_resolve_cache(&payload(other));
    assert!(found.success, "{:?}", found.error);
    assert_eq!(
        std::fs::read(found.local_path.unwrap()).unwrap(),
        theirs,
        "the file of the other process is used, not decrypted again"
    );

    // and removes the first one: it is decrypted again
    std::fs::remove_file(&path).unwrap();
    let again = hub.image_decrypt(&ImagePayload {
        force: true,
        ..payload(MD5)
    });
    assert!(again.success, "{:?}", again.error);
    assert_eq!(std::fs::read(&path).unwrap(), mine);
}

#[test]
fn reports_missing_files_keys_and_bad_keys() {
    let (_root, hub, _account, img_dir) = setup("img-errors");
    let p = ImagePayload {
        session_id: Some("wxid_bob".into()),
        image_md5: Some(MD5.into()),
        create_time: Some(1_700_000_000),
        prefer_file_path: true,
        ..Default::default()
    };
    let missing = hub.image_decrypt(&p);
    assert!(!missing.success);
    assert_eq!(missing.failure_kind, Some("not_found"));
    assert!(hub
        .image_decrypt(&ImagePayload::default())
        .error
        .unwrap()
        .contains("missing image identifier"));

    // wrong AES key: the strict padding check rejects it
    let plain = jpeg(2000, 5);
    std::fs::write(
        img_dir.join(format!("{MD5}.dat")),
        encrypt_v2(&plain, b"ffffffffffffffff", 0x5a),
    )
    .unwrap();
    let bad = hub.image_decrypt(&p);
    assert!(!bad.success);
    assert!(bad.error.unwrap().contains("decrypt failed"));

    // no xor key configured at all
    let (_root2, hub2) = common::mock_hub("img-nokeys");
    let r = hub2.image_decrypt(&p);
    assert!(!r.success);
}

/// Two image messages from 2023-11-14 in `wxid_bob`'s chat, image keys configured; returns the hub and the
/// `Img` directory where `.dat` files belong.
fn export_world(
    tag: &str,
) -> (
    TestDir,
    weflow_core::services::ServiceHub,
    std::path::PathBuf,
) {
    use weflow_native::fixture::{MsgSpec, SessionSpec, T0};
    let (root, hub, fixture) = common::custom_hub_with(
        tag,
        |f| {
            f.session_db(&[SessionSpec {
                username: "wxid_bob",
                summary: "",
                last_timestamp: T0,
                unread: 0,
                last_msg_type: 3,
            }]);
            let image = |id: i64, at: i64, md5: &str| {
                MsgSpec::text(
                    id,
                    "wxid_bob",
                    T0 + at,
                    &format!("<msg><img md5=\"{md5}\"/></msg>"),
                )
                .of_type(3)
            };
            f.message_shard(
                0,
                &[(
                    "wxid_bob",
                    vec![
                        image(1, 0, "aabbccddeeff00112233445566778899"),
                        image(2, 100, "11223344556677889900aabbccddeeff"),
                    ],
                )],
            );
        },
        |p| {
            p.image_xor_key = Some(0x5a);
            p.image_aes_key = Some(KEY.into());
        },
    );
    let month = weflow_core::image::year_month_from_create_time(Some(1_700_000_000));
    let img_dir = fixture
        .account_dir
        .join("msg/attach")
        .join(md5_hex("wxid_bob"))
        .join(&month)
        .join("Img");
    std::fs::create_dir_all(&img_dir).unwrap();
    (root, hub, img_dir)
}

#[tokio::test]
async fn export_media_copies_images_of_a_conversation() {
    let (root, hub, img_dir) = export_world("img-export");
    let md5 = "aabbccddeeff00112233445566778899";
    let plain = jpeg(3000, 7);
    std::fs::write(
        img_dir.join(format!("{md5}_h.dat")),
        encrypt_v2(&plain, KEY.as_bytes().try_into().unwrap(), 0x5a),
    )
    .unwrap();

    let out = root.join("media-out");
    let r = serde_json::to_value(
        hub.export_media(Some("wxid_bob"), &out, "image", None, None)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(r["exported"], 1, "{r}");
    assert_eq!(r["found"], 2, "two image messages, one has no file on disk");
    assert_eq!(r["missing"], 1);
    assert_eq!(r["thumbOnly"], 0, "the exported file is the HD original");
    let path = r["files"][0]["path"].as_str().unwrap();
    assert!(
        path.contains("wxid_bob") && path.ends_with(&format!("{md5}.jpg")),
        "{path}"
    );
    assert_eq!(std::fs::read(path).unwrap(), plain);

    assert!(hub
        .export_media(Some("wxid_bob"), &out, "bogus", None, None)
        .await
        .is_err());
}

/// The same image (md5) shown by two messages: the older one, from a month whose folder has no file, and the newer
/// one, whose month has it. The threads of an export reach them in either order.
fn shared_image_world(tag: &str) -> (TestDir, weflow_core::services::ServiceHub, Vec<u8>) {
    use weflow_native::fixture::{MsgSpec, SessionSpec, DAY, T0};
    let md5 = "aabbccddeeff00112233445566778899";
    let (root, hub, fixture) = common::custom_hub_with(
        tag,
        |f| {
            f.session_db(&[SessionSpec {
                username: "wxid_bob",
                summary: "",
                last_timestamp: T0,
                unread: 0,
                last_msg_type: 3,
            }]);
            let image = |id: i64, at: i64| {
                MsgSpec::text(
                    id,
                    "wxid_bob",
                    at,
                    &format!("<msg><img md5=\"{md5}\"/></msg>"),
                )
                .of_type(3)
            };
            f.message_shard(
                0,
                &[("wxid_bob", vec![image(1, T0 - 40 * DAY), image(2, T0)])],
            );
        },
        |p| {
            p.image_xor_key = Some(0x5a);
            p.image_aes_key = Some(KEY.into());
        },
    );
    let month = weflow_core::image::year_month_from_create_time(Some(1_700_000_000));
    let img_dir = fixture
        .account_dir
        .join("msg/attach")
        .join(md5_hex("wxid_bob"))
        .join(&month)
        .join("Img");
    std::fs::create_dir_all(&img_dir).unwrap();
    let plain = jpeg(3000, 11);
    std::fs::write(
        img_dir.join(format!("{md5}_h.dat")),
        encrypt_v2(&plain, KEY.as_bytes().try_into().unwrap(), 0x5a),
    )
    .unwrap();
    (root, hub, plain)
}

#[tokio::test]
async fn a_shared_image_is_exported_for_every_message_whatever_the_thread_order() {
    // the order the two messages are looked at in is up to the threads: run it a few times
    for round in 0..5 {
        let (root, hub, plain) = shared_image_world(&format!("img-shared-{round}"));
        let r = serde_json::to_value(
            hub.export_media(Some("wxid_bob"), &root.join("media"), "image", None, None)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            (r["exported"].as_i64(), r["missing"].as_i64()),
            (Some(2), Some(0)),
            "round {round}: {r}"
        );
        let files = r["files"].as_array().unwrap();
        assert_eq!(files[0]["path"], files[1]["path"], "one file for both");
        assert_eq!(
            std::fs::read(files[0]["path"].as_str().unwrap()).unwrap(),
            plain
        );

        let (root, hub, _) = shared_image_world(&format!("img-shared-msgs-{round}"));
        let opts = weflow_core::api::ApiMediaOptions {
            enabled: true,
            images: true,
            ..Default::default()
        };
        let dir = root.join("exp");
        std::fs::create_dir_all(&dir).unwrap();
        let r = serde_json::to_value(
            hub.export_messages_with_media(&media_request("json"), &dir.join("chat.json"), &opts)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            (
                r["media"]["exported"].as_i64(),
                r["media"]["missing"].as_i64()
            ),
            (Some(2), Some(0)),
            "round {round}: {r}"
        );
    }
}

#[tokio::test]
async fn a_wxgf_image_the_built_in_decoder_cannot_read_says_why_and_is_counted_missing() {
    let md5 = "aabbccddeeff00112233445566778899";
    // a WXGF blob without any HEVC picture in it
    let wxgf = {
        let mut blob = b"wxgf".to_vec();
        blob.extend([0x11u8; 1300]);
        blob
    };
    let put = |img_dir: &std::path::Path| {
        std::fs::write(
            img_dir.join(format!("{md5}_h.dat")),
            encrypt_v2(&wxgf, KEY.as_bytes().try_into().unwrap(), 0x5a),
        )
        .unwrap();
    };

    let (root, hub, img_dir) = export_world("img-wxgf-unsupported");
    put(&img_dir);
    let p = ImagePayload {
        session_id: Some("wxid_bob".into()),
        image_md5: Some(md5.into()),
        create_time: Some(1_700_000_000),
        prefer_file_path: true,
        ..Default::default()
    };
    let r = hub.image_decrypt(&p);
    assert!(!r.success);
    assert_eq!(r.failure_kind, Some("unsupported_image"));
    let sent = weflow_core::message::time::format_timestamp(1_700_000_000);
    assert_eq!(
        r.error.unwrap(),
        format!("WXGF image sent {sent} cannot be converted: the data is damaged (no picture)")
    );
    assert!(
        !hub.image_resolve_cache(&p).success,
        "nothing was cached from the unconverted file"
    );

    // an export: the image is missing like one whose file is not on disk, and nothing else is said about it
    let r = serde_json::to_value(
        hub.export_media(Some("wxid_bob"), &root.join("media"), "image", None, None)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (r["exported"].as_i64(), r["missing"].as_i64()),
        (Some(0), Some(2)),
        "{r}"
    );
    assert!(
        ["ffmpegMissing", "ffmpegMissingImages", "hint"]
            .iter()
            .all(|k| r.get(k).is_none()),
        "{r}"
    );

    let (root2, hub, img_dir) = export_world("img-wxgf-unsupported-msgs");
    put(&img_dir);
    let opts = weflow_core::api::ApiMediaOptions {
        enabled: true,
        images: true,
        ..Default::default()
    };
    let dir = root2.join("exp");
    std::fs::create_dir_all(&dir).unwrap();
    let r = serde_json::to_value(
        hub.export_messages_with_media(&media_request("json"), &dir.join("chat.json"), &opts)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(r["media"]["exported"], 0, "{r}");
    assert!(r["media"].get("ffmpegMissing").is_none(), "{r}");

    // a WXGF picture the built-in decoder reads
    let (root3, hub, img_dir) = export_world("img-wxgf-builtin");
    let mut blob = b"wxgf".to_vec();
    blob.extend([0x24u8, 0x01, 0x00, 0x46, 0x00, 0x2e]); // a header, before the HEVC stream
    blob.extend(include_bytes!("../testdata/wxgf/full_range.hevc"));
    std::fs::write(
        img_dir.join(format!("{md5}_h.dat")),
        encrypt_v2(&blob, KEY.as_bytes().try_into().unwrap(), 0x5a),
    )
    .unwrap();
    let r = hub.image_decrypt(&p);
    assert!(r.success, "{:?}", r.error);
    let jpg = std::fs::read(r.local_path.unwrap()).unwrap();
    let mut decoder = jpeg_decoder::Decoder::new(&jpg[..]);
    decoder.read_info().unwrap();
    let info = decoder.info().unwrap();
    assert_eq!((info.width, info.height), (70, 46));
    let r = serde_json::to_value(
        hub.export_media(Some("wxid_bob"), &root3.join("media"), "image", None, None)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (r["exported"].as_i64(), r["missing"].as_i64()),
        (Some(1), Some(1)),
        "{r}"
    );
}

#[tokio::test]
async fn export_media_reports_thumbnail_only_images() {
    let (root, hub, img_dir) = export_world("img-export-thumb");
    let md5 = "aabbccddeeff00112233445566778899";
    // only the thumbnail rendition exists on disk
    let thumb = jpeg(1500, 9);
    std::fs::write(
        img_dir.join(format!("{md5}_t.dat")),
        encrypt_v2(&thumb, KEY.as_bytes().try_into().unwrap(), 0x5a),
    )
    .unwrap();
    let r = serde_json::to_value(
        hub.export_media(
            Some("wxid_bob"),
            &root.join("media-thumb"),
            "image",
            None,
            None,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (r["exported"].as_i64(), r["thumbOnly"].as_i64()),
        (Some(1), Some(1)),
        "{r}"
    );
    assert!(
        r["note"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("thumbnail"),
        "the note explains what thumbOnly means"
    );
    assert_eq!(r["files"][0]["isThumb"], true);
}

#[tokio::test]
async fn export_media_honours_the_date_range_and_reports_missing_by_kind() {
    let (root, hub, _img_dir) = export_world("img-export-range");
    let out = root.join("media-range");
    // both image messages are from 2023-11-14; a later window matches nothing
    let none = serde_json::to_value(
        hub.export_media(Some("wxid_bob"), &out, "image", Some(1_800_000_000), None)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(none["found"], 0);
    let all = serde_json::to_value(
        hub.export_media(
            Some("wxid_bob"),
            &out,
            "image",
            Some(1_600_000_000),
            Some(1_800_000_000),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(all["found"], 2);
    assert_eq!(all["missing"], 2, "no .dat files exist");
    assert_eq!(all["missingByKind"]["image"], 2);
}

/// bob's chat: text, an image on disk, an image that was never downloaded, a voice message and a sticker.
fn embed_world(tag: &str) -> (TestDir, weflow_core::services::ServiceHub) {
    use weflow_native::fixture::{MsgSpec, SessionSpec, VoiceSpec, T0};
    let md5 = "aabbccddeeff00112233445566778899";
    let (root, hub, fixture) = common::custom_hub_with(
        tag,
        |f| {
            f.session_db(&[SessionSpec {
                username: "wxid_bob",
                summary: "",
                last_timestamp: T0,
                unread: 0,
                last_msg_type: 3,
            }]);
            let image = |id: i64, at: i64, md5: &str| {
                MsgSpec::text(
                    id,
                    "wxid_bob",
                    T0 + at,
                    &format!("<msg><img md5=\"{md5}\"/></msg>"),
                )
                .of_type(3)
            };
            f.message_shard(
                0,
                &[(
                    "wxid_bob",
                    vec![
                        MsgSpec::text(1, "wxid_bob", T0, "hello"),
                        image(2, 10, md5),
                        image(3, 20, "11223344556677889900aabbccddeeff"),
                        MsgSpec::text(4, "wxid_me", T0 + 200, "<voicemsg length=\"1000\" voicelength=\"1000\"/>").of_type(34),
                        MsgSpec::text(5, "wxid_bob", T0 + 300, "<msg><emoji md5=\"00112233445566778899aabbccddeeff\" cdnurl=\"http://127.0.0.1:9/x\"/></msg>").of_type(47),
                    ],
                )],
            );
            let silk = weflow_silk::testenc::encode_tone(24000);
            f.media_db(
                0,
                &[VoiceSpec {
                    chat: "wxid_bob",
                    create_time: T0 + 200,
                    local_id: 4,
                    svr_id: 1004,
                    data: silk,
                    index: "0",
                }],
            );
        },
        |p| {
            p.image_xor_key = Some(0x5a);
            p.image_aes_key = Some(KEY.into());
        },
    );
    let month = weflow_core::image::year_month_from_create_time(Some(1_700_000_000));
    let img_dir = fixture
        .account_dir
        .join("msg/attach")
        .join(md5_hex("wxid_bob"))
        .join(&month)
        .join("Img");
    std::fs::create_dir_all(&img_dir).unwrap();
    std::fs::write(
        img_dir.join(format!("{md5}_h.dat")),
        encrypt_v2(&jpeg(3000, 7), KEY.as_bytes().try_into().unwrap(), 0x5a),
    )
    .unwrap();
    (root, hub)
}

fn media_request(format: &str) -> weflow_core::services::MessageExportRequest {
    weflow_core::services::MessageExportRequest {
        session_id: "wxid_bob".into(),
        format: format.into(),
        start: None,
        end: None,
        sender: None,
        display_pref: weflow_core::export_msg::names::DisplayPref::Remark,
        excel_compact: false,
    }
}

#[tokio::test]
async fn message_exports_point_at_copied_media_in_every_format_that_has_room_for_it() {
    let (root, hub) = embed_world("embed");
    let opts = weflow_core::api::ApiMediaOptions {
        enabled: true,
        images: true,
        voices: true,
        videos: false,
        emojis: false,
        files: false,
        avatars: None,
    };
    let dir = root.join("exp");
    std::fs::create_dir_all(&dir).unwrap();
    let image_rel = "media/chat/images/aabbccddeeff00112233445566778899.jpg";
    let voice_rel = "media/chat/voices/voice_4.wav";

    let r = serde_json::to_value(
        hub.export_messages_with_media(&media_request("json"), &dir.join("chat.json"), &opts)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (
            r["media"]["requested"].as_i64(),
            r["media"]["exported"].as_i64(),
            r["media"]["missing"].as_i64()
        ),
        (Some(3), Some(2), Some(1)),
        "{r}"
    );
    assert!(
        dir.join(image_rel).exists()
            && std::fs::read(dir.join(voice_rel))
                .unwrap()
                .starts_with(b"RIFF")
    );
    assert!(
        !dir.join("media/chat/emojis").exists(),
        "stickers were not asked for"
    );
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("chat.json")).unwrap()).unwrap();
    let content = |t: i64, n: usize| {
        json["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["localType"] == t)
            .nth(n)
            .unwrap()["content"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(content(3, 0), image_rel);
    assert_eq!(
        content(3, 1),
        "[图片]",
        "an image that is not on disk keeps its placeholder"
    );
    assert_eq!(content(34, 0), voice_rel);
    assert_eq!(content(1, 0), "hello");
    assert!(
        !content(47, 0).contains("media/"),
        "stickers keep their caption text"
    );

    // chatlab only has room for images
    hub.export_messages_with_media(
        &media_request("chatlab"),
        &dir.join("chat.chatlab.json"),
        &opts,
    )
    .await
    .unwrap();
    let lab: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("chat.chatlab.json")).unwrap())
            .unwrap();
    let contents: Vec<&str> = lab["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["content"].as_str().unwrap_or(""))
        .collect();
    // the media folder is named after the output file (`chat.chatlab.json` -> `media/chat.chatlab/`)
    assert!(
        contents.contains(&"media/chat.chatlab/images/aabbccddeeff00112233445566778899.jpg")
            && !contents.iter().any(|c| c.ends_with(".wav")),
        "{contents:?}"
    );

    // txt rows, weclone `src`, html tags
    hub.export_messages_with_media(&media_request("txt"), &dir.join("chat.txt"), &opts)
        .await
        .unwrap();
    let txt = std::fs::read_to_string(dir.join("chat.txt")).unwrap();
    assert!(txt.contains(image_rel) && txt.contains(voice_rel));
    hub.export_messages_with_media(&media_request("weclone"), &dir.join("chat.csv"), &opts)
        .await
        .unwrap();
    assert!(std::fs::read_to_string(dir.join("chat.csv"))
        .unwrap()
        .contains(image_rel));
    hub.export_messages_with_media(&media_request("html"), &dir.join("chat.html"), &opts)
        .await
        .unwrap();
    let html = std::fs::read_to_string(dir.join("chat.html")).unwrap();
    assert!(
        html.contains(&format!("\"k\":2,\"s\":\"{image_rel}\""))
            && html.contains(&format!("\"k\":3,\"s\":\"{voice_rel}\"")),
        "the page's data points the picture and the voice message at their files"
    );

    // without media nothing is copied and the placeholders stay
    let plain_dir = root.join("plain");
    std::fs::create_dir_all(&plain_dir).unwrap();
    hub.export_messages_with_media(
        &media_request("json"),
        &plain_dir.join("chat.json"),
        &weflow_core::api::ApiMediaOptions::default(),
    )
    .await
    .unwrap();
    assert!(!plain_dir.join("media").exists());
}

#[tokio::test]
async fn several_formats_without_media_are_what_each_writes_alone() {
    // #628: read once and written side by side, each file as the streaming export of that format writes it
    let (root, hub) = embed_world("formats-alone");
    let (together, alone) = (root.join("together"), root.join("alone"));
    std::fs::create_dir_all(&together).unwrap();
    std::fs::create_dir_all(&alone).unwrap();
    let formats = [
        "txt",
        "json",
        "arkme-json",
        "chatlab",
        "chatlab-jsonl",
        "weclone",
        "sql",
        "html",
    ];
    let outs: Vec<(String, std::path::PathBuf)> = formats
        .iter()
        .map(|f| (f.to_string(), together.join(format!("chat.{f}"))))
        .collect();
    let results = hub
        .export_messages_formats(
            &media_request("txt"),
            &outs,
            &weflow_core::api::ApiMediaOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), formats.len());
    for (format, out) in &outs {
        let single = alone.join(out.file_name().unwrap());
        hub.export_messages(&media_request(format), &single)
            .unwrap();
        // all but the line with the time of the export, which may fall in another second
        let written = std::fs::read_to_string(out).unwrap();
        let by_itself = std::fs::read_to_string(&single).unwrap();
        assert_eq!(
            written.lines().count(),
            by_itself.lines().count(),
            "{format}"
        );
        let differing = written
            .lines()
            .zip(by_itself.lines())
            .filter(|(a, b)| a != b)
            .count();
        assert!(differing <= 1, "{format}: {differing} lines differ");
    }
}

#[tokio::test]
async fn several_formats_share_one_media_folder_and_pack_into_one_zip() {
    // #342: the messages read and the media copied once, every format pointing at the same files
    let (root, hub) = embed_world("formats-zip");
    let opts = weflow_core::api::ApiMediaOptions {
        enabled: true,
        images: true,
        voices: true,
        ..Default::default()
    };
    let dir = root.join("parts");
    std::fs::create_dir_all(&dir).unwrap();
    let outs: Vec<(String, std::path::PathBuf)> = [
        ("json", "chat.json"),
        ("html", "chat.html"),
        ("txt", "chat.txt"),
    ]
    .iter()
    .map(|(f, name)| (f.to_string(), dir.join(name)))
    .collect();
    let results = serde_json::to_value(
        hub.export_messages_formats(&media_request("json"), &outs, &opts)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(results.as_array().unwrap().len(), 3);
    let image_rel = "media/chat/images/aabbccddeeff00112233445566778899.jpg";
    assert!(dir.join(image_rel).exists());
    for (_, path) in &outs {
        assert!(path.exists(), "{}", path.display());
    }
    for name in ["chat.json", "chat.html"] {
        let text = std::fs::read_to_string(dir.join(name)).unwrap();
        assert!(text.contains(image_rel), "{name} points at the one copy");
    }
    assert_eq!(results[1]["media"], results[0]["media"]);

    let zip = root.join("chat.zip");
    weflow_core::export::zip_folder(&dir, &zip).unwrap();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap();
    let mut names: Vec<String> = archive.file_names().map(str::to_string).collect();
    names.sort();
    assert!(
        [
            "chat.html",
            "chat.json",
            "chat.txt",
            image_rel,
            "media/chat/voices/voice_4.wav"
        ]
        .iter()
        .all(|n| names.iter().any(|m| m == n)),
        "{names:?}"
    );
    let mut image = Vec::new();
    std::io::Read::read_to_end(&mut archive.by_name(image_rel).unwrap(), &mut image).unwrap();
    assert_eq!(image, std::fs::read(dir.join(image_rel)).unwrap());
}

/// WeChat's media index (`hardlink.db`) only adds to what the folders of the conversation hold (#651): a better
/// rendition it names wins, an image only it knows is found, and what the folders find is kept otherwise, also for
/// the many images the index has no row for.
#[test]
fn the_media_index_adds_to_what_the_folders_hold() {
    use weflow_native::fixture::{Fixture, HardlinkSpec};
    let (root, hub, account, img_dir) = setup("img-hardlink");
    let key: [u8; 16] = KEY.as_bytes().try_into().unwrap();
    let talker = md5_hex("wxid_bob");
    // a month folder the message's month does not name
    let elsewhere = account.join("msg/attach").join(&talker).join("2020-01/Img");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let put = |dir: &std::path::Path, name: &str, image: &[u8]| {
        std::fs::write(dir.join(name), encrypt_v2(image, &key, 0x5a)).unwrap();
    };
    // (md5, what the folders hold, what the index names elsewhere, the image expected)
    let [only_thumb, tie, no_row, only_linked] = ["thumb", "tie", "no-row", "linked"].map(md5_hex);
    let (thumb, original, listed, renamed, plain, hd) = (
        jpeg(1200, 1),
        jpeg(2400, 2),
        jpeg(2500, 3),
        jpeg(2600, 4),
        jpeg(2700, 5),
        jpeg(2800, 6),
    );
    put(&img_dir, &format!("{only_thumb}_t.dat"), &thumb);
    put(&elsewhere, &format!("{only_thumb}.dat"), &original);
    put(&img_dir, &format!("{tie}.dat"), &listed);
    put(&elsewhere, "0123456789abcdef0123456789abcdef.dat", &renamed);
    put(&img_dir, &format!("{no_row}.dat"), &plain);
    put(&elsewhere, &format!("{only_linked}_h.dat"), &hd);
    let file = |md5: &str| format!("{md5}.dat");
    let hd_file = format!("{only_linked}_h.dat");
    Fixture::open(&root.join("data"), "wxid_me_ab12").hardlink_db(&[
        HardlinkSpec::image(&only_thumb, &file(&only_thumb), &talker, "2020-01"),
        HardlinkSpec::image(
            &tie,
            "0123456789abcdef0123456789abcdef.dat",
            &talker,
            "2020-01",
        ),
        HardlinkSpec::image(&only_linked, &hd_file, &talker, "2020-01"),
        // a row whose file is gone changes nothing
        HardlinkSpec::image(
            &no_row,
            "fedcba9876543210fedcba9876543210_h.dat",
            &talker,
            "2020-01",
        ),
    ]);

    for (md5, expected, is_thumb, why) in [
        (
            &only_thumb,
            &original,
            false,
            "the index's original beats the folder's thumbnail",
        ),
        (
            &tie,
            &listed,
            false,
            "what the folders find is kept when the index has nothing better",
        ),
        (
            &no_row,
            &plain,
            false,
            "an image the index has no file for is found in the folders",
        ),
        (
            &only_linked,
            &hd,
            false,
            "an image only the index knows is found",
        ),
    ] {
        let r = hub.image_decrypt(&ImagePayload {
            session_id: Some("wxid_bob".into()),
            image_md5: Some(md5.clone()),
            create_time: Some(1_700_000_000),
            prefer_file_path: true,
            ..Default::default()
        });
        assert!(r.success, "{why}: {:?}", r.error);
        assert_eq!(
            &std::fs::read(r.local_path.unwrap()).unwrap(),
            expected,
            "{why}"
        );
        assert_eq!(r.is_thumb, Some(is_thumb), "{why}");
    }

    // the index is read whole once, and again when WeChat changes it
    let later = md5_hex("later");
    let later_image = jpeg(2900, 7);
    put(&elsewhere, &file(&later), &later_image);
    let later_payload = ImagePayload {
        session_id: Some("wxid_bob".into()),
        image_md5: Some(later.clone()),
        create_time: Some(1_700_000_000),
        prefer_file_path: true,
        ..Default::default()
    };
    assert_eq!(
        hub.image_decrypt(&later_payload).failure_kind,
        Some("not_found")
    );
    Fixture::open(&root.join("data"), "wxid_me_ab12").hardlink_db(&[HardlinkSpec::image(
        &later,
        &file(&later),
        &talker,
        "2020-01",
    )]);
    let r = hub.image_decrypt(&later_payload);
    assert!(r.success, "{:?}", r.error);
    assert_eq!(std::fs::read(r.local_path.unwrap()).unwrap(), later_image);
}

/// A video whose file is named otherwise than the message's md5 is not in the month folders under that name; WeChat's
/// media index says where it is (#651). A video the folders find stays the one they find.
#[test]
fn a_video_the_folders_miss_is_found_through_the_media_index() {
    use weflow_native::fixture::{Fixture, HardlinkSpec, MsgSpec};
    let (root, hub, account, _img_dir) = setup("video-hardlink");
    let at = 1_700_000_000;
    let month = weflow_core::image::year_month_from_create_time(Some(at));
    let folder = account.join("msg/video").join(&month);
    std::fs::create_dir_all(&folder).unwrap();
    let (renamed, listed) = (md5_hex("renamed"), md5_hex("listed"));
    let xml =
        |md5: &str| format!("<msg><videomsg md5=\"{md5}\" length=\"9\" playlength=\"1\" /></msg>");
    let fixture = Fixture::open(&root.join("data"), "wxid_me_ab12");
    fixture.message_shard(
        2,
        &[(
            "wxid_bob",
            vec![
                MsgSpec::text(20, "wxid_bob", at, &xml(&renamed)).of_type(43),
                MsgSpec::text(21, "wxid_bob", at + 1, &xml(&listed)).of_type(43),
            ],
        )],
    );
    // the first is on disk under another name, with its cover beside it; the second under its md5
    let other = "ffeeddccbbaa99887766554433221100";
    std::fs::write(folder.join(format!("{other}.mp4")), b"renamed video").unwrap();
    std::fs::write(folder.join(format!("{other}.jpg")), b"\xFF\xD8cover").unwrap();
    std::fs::write(folder.join(format!("{listed}.mp4")), b"listed video").unwrap();
    std::fs::write(
        folder.join("00112233445566778899aabbccddeeff.mp4"),
        b"elsewhere",
    )
    .unwrap();
    let other_file = format!("{other}.mp4");
    fixture.hardlink_db(&[
        HardlinkSpec::video(&renamed, &other_file, &month),
        HardlinkSpec::video(&listed, "00112233445566778899aabbccddeeff.mp4", &month),
    ]);

    let found = hub
        .video_files_for_message("wxid_bob", "20", Some(at))
        .unwrap();
    assert_eq!(std::fs::read(&found.video).unwrap(), b"renamed video");
    assert_eq!(found.cover, Some(folder.join(format!("{other}.jpg"))));
    let info = hub
        .video_info(&renamed, true, weflow_core::video::PosterFormat::DataUrl)
        .unwrap();
    assert!(info.exists);
    assert!(info
        .cover_url
        .unwrap()
        .starts_with("data:image/jpeg;base64,"));

    let kept = hub
        .video_files_for_message("wxid_bob", "21", Some(at + 1))
        .unwrap();
    assert_eq!(std::fs::read(&kept.video).unwrap(), b"listed video");
}

/// A file message's file is copied below `files/<month>/`, as WeChat keeps it: found through WeChat's media index,
/// else by its name in the month it was sent. Files of one name from different months stay apart, a file sent twice
/// is copied once, and one not on disk is counted missing (#694).
#[tokio::test]
async fn file_messages_copy_their_files() {
    use weflow_native::fixture::{HardlinkSpec, MsgSpec, SessionSpec, T0};
    let (indexed, unindexed, gone) = (md5_hex("indexed"), md5_hex("unindexed"), md5_hex("gone"));
    let file = |name: &str, md5: &str| {
        format!(
            "<msg><appmsg><title>{name}</title><type>6</type><appattach><totallen>5</totallen>\
             <fileext>pdf</fileext></appattach><md5>{md5}</md5></appmsg></msg>"
        )
    };
    // the second is sent a month and a half later, in another month folder
    let later = T0 + 45 * 86_400;
    let (root, hub, fixture) = common::custom_hub_with(
        "files",
        |f| {
            f.session_db(&[SessionSpec {
                username: "wxid_bob",
                summary: "",
                last_timestamp: later,
                unread: 0,
                last_msg_type: 49,
            }]);
            f.message_shard(
                0,
                &[(
                    "wxid_bob",
                    vec![
                        MsgSpec::text(1, "wxid_bob", T0, &file("report.pdf", &indexed)).of_type(49),
                        MsgSpec::text(2, "wxid_bob", later, &file("report.pdf", &unindexed))
                            .of_type(49),
                        MsgSpec::text(3, "wxid_bob", later + 1, &file("gone.pdf", &gone))
                            .of_type(49),
                        MsgSpec::text(4, "wxid_me", later + 2, &file("report.pdf", &indexed))
                            .of_type(49),
                        MsgSpec::text(5, "wxid_bob", later + 3, "hello"),
                    ],
                )],
            );
        },
        |_| {},
    );
    let month = |t: i64| weflow_core::image::year_month_from_create_time(Some(t));
    let (first, second) = (month(T0), month(later));
    assert_ne!(first, second);
    // the indexed one lies in a folder its send time does not name
    let files = fixture.account_dir.join("msg/file");
    for (dir, body) in [("2019-01", "first"), (second.as_str(), "second")] {
        std::fs::create_dir_all(files.join(dir)).unwrap();
        std::fs::write(files.join(dir).join("report.pdf"), body).unwrap();
    }
    fixture.hardlink_db(&[HardlinkSpec::file(&indexed, "report.pdf", "2019-01")]);

    let dir = root.join("exp");
    let opts = weflow_core::api::ApiMediaOptions {
        enabled: true,
        files: true,
        ..Default::default()
    };
    let r = serde_json::to_value(
        hub.export_messages_with_media(&media_request("html"), &dir.join("chat.html"), &opts)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (
            r["media"]["requested"].as_i64(),
            r["media"]["exported"].as_i64(),
            r["media"]["missing"].as_i64()
        ),
        (Some(4), Some(3), Some(1)),
        "{r}"
    );
    let copied =
        |rel: &str| std::fs::read_to_string(dir.join("media/chat/files").join(rel)).unwrap();
    assert_eq!(copied("2019-01/report.pdf"), "first");
    assert_eq!(copied(&format!("{second}/report.pdf")), "second");

    let html = std::fs::read_to_string(dir.join("chat.html")).unwrap();
    let open = "<script id=\"wf-data\" type=\"application/json\">";
    let start = html.find(open).unwrap() + open.len();
    let end = start + html[start..].find("</script>").unwrap();
    let data: serde_json::Value = serde_json::from_str(&html[start..end]).unwrap();
    let msgs = data["msgs"].as_array().unwrap();
    let paths: Vec<Option<&str>> = msgs.iter().map(|m| m["s"].as_str()).collect();
    assert_eq!(
        paths,
        [
            Some("media/chat/files/2019-01/report.pdf"),
            Some(format!("media/chat/files/{second}/report.pdf").as_str()),
            None,
            Some("media/chat/files/2019-01/report.pdf"),
            None,
        ]
    );
    assert!(
        msgs[..4].iter().all(|m| m["k"] == 6 && m["n"].is_string()),
        "{msgs:?}"
    );
    assert_eq!(msgs[2]["n"], "gone.pdf", "a file not copied keeps its card");

    // `export media --type file`: the same files, below the conversation's folder
    let out = root.join("media-out");
    let r = hub
        .export_media(Some("wxid_bob"), &out, "file", None, None)
        .await
        .unwrap();
    let r = serde_json::to_value(r).unwrap();
    assert_eq!(
        (
            r["found"].as_i64(),
            r["exported"].as_i64(),
            r["missing"].as_i64()
        ),
        (Some(4), Some(3), Some(1)),
        "{r}"
    );
    assert_eq!(
        std::fs::read_to_string(out.join("wxid_bob/files/2019-01/report.pdf")).unwrap(),
        "first"
    );
}

/// The avatars on the HTML page (#700): WeChat's copy on this computer first; the others from the contact's
/// avatar address unless asked not to download; who has none keeps the page's initial.
#[tokio::test]
async fn html_pages_carry_the_avatars() {
    use base64::Engine as _;
    use weflow_native::fixture::{ContactSpec, MsgSpec, SessionSpec, T0};
    let (local, remote) = (jpeg(200, 1), jpeg(300, 2));
    let address = format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&remote)
    );
    let (root, hub, fixture) = common::custom_hub_with(
        "avatars",
        |f| {
            f.session_db(&[SessionSpec {
                username: "wxid_bob",
                summary: "",
                last_timestamp: T0 + 2,
                unread: 0,
                last_msg_type: 1,
            }]);
            f.message_shard(
                0,
                &[(
                    "wxid_bob",
                    vec![
                        MsgSpec::text(1, "wxid_bob", T0, "hi"),
                        MsgSpec::text(2, "wxid_me", T0 + 1, "hello"),
                        MsgSpec::text(3, "wxid_bob", T0 + 2, "bye"),
                    ],
                )],
            );
        },
        |_| {},
    );
    // the head image is kept for Bob; the user has only an address
    fixture.head_image_db(&[("wxid_bob", &local)]);
    let me = ContactSpec {
        avatar: &address,
        ..ContactSpec::new("wxid_me", 1, "Me")
    };
    fixture.contact_db(&[ContactSpec::new("wxid_bob", 1, "Bob"), me], &[]);

    let users = |html: &str| -> Vec<(String, String)> {
        let open = "<script id=\"wf-data\" type=\"application/json\">";
        let start = html.find(open).unwrap() + open.len();
        let end = start + html[start..].find("</script>").unwrap();
        let data: serde_json::Value = serde_json::from_str(&html[start..end]).unwrap();
        data["users"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| {
                (
                    u[0].as_str().unwrap().to_string(),
                    u[1].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    let export = |name: &'static str, source| {
        let dir = root.join(name);
        let hub = &hub;
        async move {
            let opts = weflow_core::api::ApiMediaOptions {
                enabled: true,
                avatars: Some(source),
                ..Default::default()
            };
            hub.export_messages_with_media(&media_request("html"), &dir.join("chat.html"), &opts)
                .await
                .unwrap();
            (
                dir.clone(),
                std::fs::read_to_string(dir.join("chat.html")).unwrap(),
            )
        }
    };

    let (dir, html) = export("downloaded", weflow_core::api::AvatarSource::Download).await;
    let got = users(&html);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(
        got.iter()
            .all(|(_, path)| path.starts_with("media/chat/avatars/") && path.ends_with(".jpg")),
        "{got:?}"
    );
    let files: Vec<Vec<u8>> = got
        .iter()
        .map(|(_, p)| std::fs::read(dir.join(p)).unwrap())
        .collect();
    assert!(
        files.contains(&local) && files.contains(&remote),
        "Bob's kept copy and the user's downloaded one"
    );

    let (_, html) = export("kept", weflow_core::api::AvatarSource::Local).await;
    let got = users(&html);
    let with: Vec<&str> = got
        .iter()
        .filter(|(_, p)| !p.is_empty())
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(
        with.len(),
        1,
        "only Bob's kept copy, nothing downloaded: {got:?}"
    );
}

/// The lists' HTML pages (#704): `export sessions` names each conversation after its `username`, and the avatars of
/// a list's rows go to `media/<out name>/avatars/` beside the page, WeChat's copy on this computer first.
#[tokio::test]
async fn list_pages_name_the_sessions_and_carry_their_avatars() {
    use weflow_native::fixture::{ContactSpec, SessionSpec, T0};
    let local = jpeg(200, 3);
    let (root, hub, fixture) = common::custom_hub_with(
        "list-avatars",
        |f| {
            let session = |username, at| SessionSpec {
                username,
                summary: "",
                last_timestamp: at,
                unread: 0,
                last_msg_type: 1,
            };
            f.session_db(&[session("wxid_bob", T0 + 1), session("wxid_stranger", T0)]);
        },
        |_| {},
    );
    fixture.head_image_db(&[("wxid_bob", &local)]);
    fixture.contact_db(&[ContactSpec::new("wxid_bob", 1, "Bob")], &[]);

    let sessions = hub.sessions_with_names().unwrap();
    let rows = sessions.as_array().unwrap();
    let name_of = |user: &str| {
        let row = rows.iter().find(|r| r["username"] == user).unwrap();
        let keys: Vec<&String> = row.as_object().unwrap().keys().collect();
        assert_eq!(keys[..2], ["username", "display_name"], "{row}");
        row["display_name"].clone()
    };
    assert_eq!(name_of("wxid_bob"), "Bob");
    assert_eq!(
        name_of("wxid_stranger"),
        "wxid_stranger",
        "no contact: the username"
    );

    let out = root.join("lists").join("sessions.html");
    let users = vec!["wxid_bob".to_string(), "wxid_stranger".to_string()];
    let avatars = hub
        .export_list_avatars(users, &out, weflow_core::api::AvatarSource::Local)
        .await
        .unwrap();
    assert_eq!(avatars.len(), 1, "only Bob's kept copy: {avatars:?}");
    let path = &avatars["wxid_bob"];
    assert!(path.starts_with("media/sessions/avatars/"), "{path}");
    assert_eq!(std::fs::read(root.join("lists").join(path)).unwrap(), local);
}
