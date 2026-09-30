mod common;

use aes::cipher::{generic_array::GenericArray, BlockEncryptMut, KeyInit};
use md5::{Digest, Md5};
use weflow_core::services::ImagePayload;

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
    padded.extend(std::iter::repeat(pad as u8).take(pad));
    let mut enc = ecb::Encryptor::<aes::Aes128>::new(key.into());
    let mut out = vec![0x07, 0x08, 0x56, 0x32, 0x08, 0x07];
    out.extend_from_slice(&(aes_len as i32).to_le_bytes());
    out.extend_from_slice(&(xor_len as i32).to_le_bytes());
    out.push(1);
    for chunk in padded.chunks(16) {
        let mut b = GenericArray::clone_from_slice(chunk);
        enc.encrypt_block_mut(&mut b);
        out.extend_from_slice(&b);
    }
    out.extend_from_slice(&plain[aes_len..plain.len() - xor_len]);
    out.extend(plain[plain.len() - xor_len..].iter().map(|b| b ^ xor));
    out
}

fn md5_hex(s: &str) -> String {
    Md5::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn setup(tag: &str) -> (weflow_core::services::ServiceHub, std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let (hub, root) = common::mock_hub_with(tag, |p| {
        p.image_xor_key = Some(0x5a);
        p.image_aes_key = Some(KEY.into());
    });
    let account = root.join("data/wxid_me_ab12");
    let month = weflow_core::image::year_month_from_create_time(Some(1_700_000_000));
    let img_dir = account.join("msg/attach").join(md5_hex("wxid_bob")).join(&month).join("Img");
    std::fs::create_dir_all(&img_dir).unwrap();
    (hub, root, account, img_dir)
}

#[test]
fn decrypts_the_hd_variant_and_caches_it() {
    let (hub, root, _account, img_dir) = setup("img-hd");
    let hd = jpeg(3000, 1);
    let thumb = jpeg(1500, 2);
    std::fs::write(img_dir.join(format!("{MD5}_h.dat")), encrypt_v2(&hd, KEY.as_bytes().try_into().unwrap(), 0x5a)).unwrap();
    std::fs::write(img_dir.join(format!("{MD5}_t.dat")), encrypt_v2(&thumb, KEY.as_bytes().try_into().unwrap(), 0x5a)).unwrap();

    let p = ImagePayload { session_id: Some("wxid_bob".into()), image_md5: Some(MD5.into()), create_time: Some(1_700_000_000), prefer_file_path: true, ..Default::default() };
    assert_eq!(hub.image_resolve_cache(&p).failure_kind, Some("not_found"), "nothing cached yet");

    let r = hub.image_decrypt(&p);
    assert!(r.success, "{:?}", r.error);
    let path = r.local_path.unwrap();
    assert!(path.ends_with(&format!("{MD5}_hd.jpg")), "{path}");
    assert!(path.contains("Images/wxid_bob/"), "{path}");
    assert_eq!(std::fs::read(&path).unwrap(), hd, "HD rendition wins over the thumbnail");
    assert_eq!(r.is_thumb, Some(false));

    // resolving from the cache now succeeds without touching the .dat files
    let again = hub.image_resolve_cache(&p);
    assert!(again.success);
    assert_eq!(again.local_path.as_deref(), Some(path.as_str()));
    assert_eq!(again.has_update, Some(false));

    // without preferFilePath the result is a data URL
    let data = hub.image_resolve_cache(&ImagePayload { prefer_file_path: false, ..p.clone() });
    assert!(data.local_path.unwrap().starts_with("data:image/jpeg;base64,"));

    // batch: one row per payload, duplicates answered identically
    let batch = hub.image_resolve_cache_batch(&[p.clone(), p.clone(), ImagePayload { image_md5: Some("f".repeat(32)), ..Default::default() }]);
    assert_eq!(batch["rows"].as_array().unwrap().len(), 3);
    assert_eq!(batch["rows"][0], batch["rows"][1]);
    assert_eq!(batch["rows"][2]["success"], false);

    let cleared = hub.image_clear_cache();
    assert_eq!(cleared["success"], true);
    assert!(!std::path::Path::new(&path).exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn falls_back_to_the_thumbnail_and_promotes_it_when_hd_appears() {
    let (hub, root, _account, img_dir) = setup("img-thumb");
    let thumb = jpeg(1500, 3);
    let key: [u8; 16] = KEY.as_bytes().try_into().unwrap();
    std::fs::write(img_dir.join(format!("{MD5}_t.dat")), encrypt_v2(&thumb, &key, 0x5a)).unwrap();

    let p = ImagePayload { session_id: Some("wxid_bob".into()), image_md5: Some(MD5.into()), create_time: Some(1_700_000_000), prefer_file_path: true, ..Default::default() };
    let r = hub.image_decrypt(&p);
    assert!(r.success, "{:?}", r.error);
    let path = r.local_path.unwrap();
    assert!(path.ends_with(&format!("{MD5}_t.jpg")), "{path}");
    assert_eq!(r.is_thumb, Some(true));

    // force with no HD file still succeeds, falling back to the thumbnail
    let forced = hub.image_decrypt(&ImagePayload { force: true, ..p.clone() });
    assert!(forced.success);

    // the HD file shows up later: resolving the cache upgrades the thumbnail in place
    let hd = jpeg(3000, 4);
    std::fs::write(img_dir.join(format!("{MD5}_h.dat")), encrypt_v2(&hd, &key, 0x5a)).unwrap();
    let upgraded = hub.image_resolve_cache(&p);
    assert!(upgraded.success, "{:?}", upgraded.error);
    let new_path = upgraded.local_path.unwrap();
    assert!(new_path.ends_with(&format!("{MD5}_hd.jpg")), "{new_path}");
    assert_eq!(std::fs::read(&new_path).unwrap(), hd);
    assert!(!std::path::Path::new(&path).exists(), "the thumbnail cache file is removed after the upgrade");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn reports_missing_files_keys_and_bad_keys() {
    let (hub, root, _account, img_dir) = setup("img-errors");
    let p = ImagePayload { session_id: Some("wxid_bob".into()), image_md5: Some(MD5.into()), create_time: Some(1_700_000_000), prefer_file_path: true, ..Default::default() };
    let missing = hub.image_decrypt(&p);
    assert!(!missing.success);
    assert_eq!(missing.failure_kind, Some("not_found"));
    assert!(hub.image_decrypt(&ImagePayload::default()).error.unwrap().contains("missing image identifier"));

    // wrong AES key: the strict padding check rejects it
    let plain = jpeg(2000, 5);
    std::fs::write(img_dir.join(format!("{MD5}.dat")), encrypt_v2(&plain, b"ffffffffffffffff", 0x5a)).unwrap();
    let bad = hub.image_decrypt(&p);
    assert!(!bad.success);
    assert!(bad.error.unwrap().contains("decrypt failed"));

    // no xor key configured at all
    let (hub2, root2) = common::mock_hub("img-nokeys");
    let r = hub2.image_decrypt(&p);
    assert!(!r.success);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(root2);
}

/// Two image messages from 2023-11-14 in `wxid_bob`'s chat, image keys configured; returns the hub and the
/// `Img` directory where `.dat` files belong.
fn export_world(tag: &str) -> (weflow_core::services::ServiceHub, std::path::PathBuf, std::path::PathBuf) {
    use weflow_native::fixture::{MsgSpec, SessionSpec, T0};
    let (hub, root, fixture) = common::custom_hub_with(
        tag,
        |f| {
            f.session_db(&[SessionSpec { username: "wxid_bob", summary: "", last_timestamp: T0, unread: 0, last_msg_type: 3 }]);
            let image = |id: i64, at: i64, md5: &str| MsgSpec::text(id, "wxid_bob", T0 + at, &format!("<msg><img md5=\"{md5}\"/></msg>")).of_type(3);
            f.message_shard(0, &[("wxid_bob", vec![image(1, 0, "aabbccddeeff00112233445566778899"), image(2, 100, "11223344556677889900aabbccddeeff")])]);
        },
        |p| {
            p.image_xor_key = Some(0x5a);
            p.image_aes_key = Some(KEY.into());
        },
    );
    let month = weflow_core::image::year_month_from_create_time(Some(1_700_000_000));
    let img_dir = fixture.account_dir.join("msg/attach").join(md5_hex("wxid_bob")).join(&month).join("Img");
    std::fs::create_dir_all(&img_dir).unwrap();
    (hub, root, img_dir)
}

#[tokio::test]
async fn export_media_copies_images_of_a_conversation() {
    let (hub, root, img_dir) = export_world("img-export");
    let md5 = "aabbccddeeff00112233445566778899";
    let plain = jpeg(3000, 7);
    std::fs::write(img_dir.join(format!("{md5}_h.dat")), encrypt_v2(&plain, KEY.as_bytes().try_into().unwrap(), 0x5a)).unwrap();

    let out = root.join("media-out");
    let r = hub.export_media(Some("wxid_bob"), &out, "image", None, None).await.unwrap();
    assert_eq!(r["exported"], 1, "{r}");
    assert_eq!(r["found"], 2, "two image messages, one has no file on disk");
    assert_eq!(r["missing"], 1);
    assert_eq!(r["thumbOnly"], 0, "the exported file is the HD original");
    let path = r["files"][0]["path"].as_str().unwrap();
    assert!(path.contains("wxid_bob") && path.ends_with(&format!("{md5}.jpg")), "{path}");
    assert_eq!(std::fs::read(path).unwrap(), plain);

    assert!(hub.export_media(Some("wxid_bob"), &out, "bogus", None, None).await.is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn export_media_reports_thumbnail_only_images() {
    let (hub, root, img_dir) = export_world("img-export-thumb");
    let md5 = "aabbccddeeff00112233445566778899";
    // only the thumbnail rendition exists on disk
    let thumb = jpeg(1500, 9);
    std::fs::write(img_dir.join(format!("{md5}_t.dat")), encrypt_v2(&thumb, KEY.as_bytes().try_into().unwrap(), 0x5a)).unwrap();
    let r = hub.export_media(Some("wxid_bob"), &root.join("media-thumb"), "image", None, None).await.unwrap();
    assert_eq!((r["exported"].as_i64(), r["thumbOnly"].as_i64()), (Some(1), Some(1)), "{r}");
    assert!(r["note"].as_str().unwrap().to_lowercase().contains("thumbnail"), "the note explains what thumbOnly means");
    assert_eq!(r["files"][0]["isThumb"], true);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn export_media_honours_the_date_range_and_reports_missing_by_kind() {
    let (hub, root, _img_dir) = export_world("img-export-range");
    let out = root.join("media-range");
    // both image messages are from 2023-11-14; a later window matches nothing
    let none = hub.export_media(Some("wxid_bob"), &out, "image", Some(1_800_000_000), None).await.unwrap();
    assert_eq!(none["found"], 0);
    let all = hub.export_media(Some("wxid_bob"), &out, "image", Some(1_600_000_000), Some(1_800_000_000)).await.unwrap();
    assert_eq!(all["found"], 2);
    assert_eq!(all["missing"], 2, "no .dat files exist");
    assert_eq!(all["missingByKind"]["image"], 2);
    let _ = std::fs::remove_dir_all(root);
}
