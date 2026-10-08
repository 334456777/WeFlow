mod common;

use std::fs::{self, FileTimes};
use std::path::Path;
use std::time::{Duration, SystemTime};

use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
use weflow_core::image_keys::{account_identities, acquire_image_keys, collect_codes};
use weflow_core::keys::{derive_image_keys, scan_templates};

fn jpeg() -> Vec<u8> {
    let mut output = Vec::new();
    let pixels: Vec<u8> = (0..32 * 32 * 3).map(|n| (n * 17 % 256) as u8).collect();
    jpeg_encoder::Encoder::new(&mut output, 90)
        .encode(&pixels, 32, 32, jpeg_encoder::ColorType::Rgb)
        .unwrap();
    output
}

fn encrypt(plain: &[u8], code: u32, identity: &str, xor_len: usize) -> Vec<u8> {
    let (xor, aes) = derive_image_keys(code.into(), identity);
    let cipher = aes::Aes128::new(GenericArray::from_slice(aes.as_bytes()));
    let mut output = vec![7, 8, 0x56, 0x32, 8, 7];
    output.extend_from_slice(&16i32.to_le_bytes());
    output.extend_from_slice(&(xor_len as i32).to_le_bytes());
    output.push(1);
    let mut block = GenericArray::clone_from_slice(&plain[..16]);
    cipher.encrypt_block(&mut block);
    output.extend_from_slice(&block);
    let mut padding = GenericArray::clone_from_slice(&[16; 16]);
    cipher.encrypt_block(&mut padding);
    output.extend_from_slice(&padding);
    output.extend_from_slice(&plain[16..plain.len() - xor_len]);
    output.extend(plain[plain.len() - xor_len..].iter().map(|b| b ^ xor));
    output
}

fn sample(dir: &Path, name: &str, code: u32, wxid: &str, xor_len: usize) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(name), encrypt(&jpeg(), code, wxid, xor_len)).unwrap();
}

#[test]
fn collection_preserves_loose_sources_and_rejects_unrelated_tokens() {
    let root = common::temp_dir("image-codes");
    let dirs = [root.join("net/kvcomm"), root.join("net_1/kvcomm")];
    for dir in &dirs {
        fs::create_dir_all(dir).unwrap();
    }
    for name in [
        "key_123_456.statistic",
        "key_000123_suffix",
        "other_999",
        "key_0_4294967296_-2_+3_１２",
    ] {
        fs::write(dirs[0].join(name), []).unwrap();
    }
    fs::create_dir_all(dirs[0].join("key_789")).unwrap();
    fs::write(dirs[1].join("key_prefix_123_tail"), []).unwrap();
    let mut input = dirs.to_vec();
    input.push(dirs[0].clone());
    input.push(root.join("missing"));
    let result = collect_codes(&input);
    assert_eq!(result.readable_directories, 2);
    assert_eq!(result.errors.len(), 1);
    assert_eq!(
        result.candidates.iter().map(|c| c.code).collect::<Vec<_>>(),
        [123, 456]
    );
    assert_eq!(result.candidates[0].sources.len(), 3);
    assert!(result
        .candidates
        .iter()
        .flat_map(|c| &c.sources)
        .all(|s| s.parsing == "loose_decimal_tokens"));
    input.reverse();
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        serde_json::to_value(collect_codes(&input)).unwrap()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_account_is_deduplicated_and_siblings_are_excluded() {
    let root = common::temp_dir("image-identities");
    fs::create_dir_all(root.join("wxid_other_suffix")).unwrap();
    let identities = account_identities(&root.join("wxid_me_suffix"), Some("wxid_me"));
    assert_eq!(identities.into_iter().collect::<Vec<_>>(), ["wxid_me"]);
    assert_eq!(
        account_identities(Path::new("/private/custom"), Some("plain-id")).len(),
        2
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn templates_select_newest_valid_after_scanning_and_report_truncation() {
    let root = common::temp_dir("image-template-budget");
    for n in 0..40 {
        let name = format!("{n:03}_t.dat");
        sample(&root, &name, 123, "wxid_me", 64);
        fs::File::options()
            .write(true)
            .open(root.join(name))
            .unwrap()
            .set_times(
                FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(n)),
            )
            .unwrap();
    }
    fs::write(root.join("bad_t.dat"), [7, 8, 0x56, 0x32, 8, 7]).unwrap();
    fs::write(root.join("other_t.dat"), b"not a V2 image").unwrap();
    fs::write(root.join("tiny_t.dat"), [7, 8]).unwrap();
    let scan = scan_templates(&root, 100, 2);
    assert!(!scan.truncated);
    assert!(scan.templates_truncated);
    assert_eq!(scan.valid_templates, 40);
    assert_eq!(scan.damaged_templates, 1);
    assert_eq!(scan.invalid_format, 2);
    assert_eq!(scan.templates[0].path.file_name().unwrap(), "039_t.dat");
    assert_eq!(scan.templates[1].path.file_name().unwrap(), "038_t.dat");
    let limited = scan_templates(&root, 4, 32);
    assert!(limited.truncated);
    assert_eq!(limited.entries_scanned, 4);
    let missing = scan_templates(&root.join("missing"), 100, 32);
    assert_eq!(missing.read_errors, 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn service_acquires_and_decodes_without_native_helpers_or_saved_image_keys() {
    let (hub, root) = common::mock_hub_with("image-key-rust-service", |p| {
        p.image_aes_key = None;
        p.image_xor_key = None;
    });
    let account = root.join("data/wxid_me_ab12");
    let dirs = [root.join("net/kvcomm"), root.join("net_1/kvcomm")];
    for dir in &dirs {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("key_123_999.statistic"), []).unwrap();
    }
    // A leftover other account code must not be selected even if its own samples exist nearby.
    fs::write(dirs[0].join("key_456_other"), []).unwrap();
    sample(
        &root.join("data/wxid_other_suffix"),
        "other_t.dat",
        456,
        "wxid_other",
        64,
    );
    sample(
        &account.join("msg/attach"),
        "current_t.dat",
        123,
        "wxid_me",
        64,
    );
    let result = hub.key_image(None, &dirs, 1000).unwrap();
    let (xor, aes) = derive_image_keys(123, "wxid_me");
    assert_eq!(result["image_aes_key"], aes);
    assert_eq!(result["image_xor_key"], xor);
    assert_eq!(result["sources"].as_array().unwrap().len(), 2);
    assert_eq!(result["aes_verified"], true);
    assert_eq!(result["xor_verified"], true);
    assert_eq!(result["verified"], true);
    let encoded = fs::read(account.join("msg/attach/current_t.dat")).unwrap();
    let key = weflow_core::decrypt::parse_aes_key(&aes).unwrap();
    let decoded = weflow_core::decrypt::decrypt_dat(&encoded, xor, Some(&key)).unwrap();
    assert_eq!(decoded.data, jpeg());
    assert!(jpeg_decoder::Decoder::new(decoded.data.as_slice())
        .decode()
        .is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn aes_header_alone_does_not_verify_xor_or_the_pair() {
    let root = common::temp_dir("image-partial-verification");
    let dir = root.join("kvcomm");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("key_123_suffix"), []).unwrap();
    let account = root.join("wxid_me_suffix");
    sample(&account, "zero-tail_t.dat", 123, "wxid_me", 0);
    let result = acquire_image_keys(&[dir], &account, Some("wxid_me"), 100).unwrap();
    assert_eq!(result["aes_verified"], true);
    assert_eq!(result["xor_verified"], false);
    assert_eq!(result["verified"], false);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_jpeg_or_wrong_xor_never_verifies_the_pair() {
    let root = common::temp_dir("image-invalid-pair");
    let dir = root.join("kvcomm");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("key_123_suffix"), []).unwrap();
    let account = root.join("wxid_me_suffix");
    fs::create_dir_all(&account).unwrap();
    let mut fake = vec![0; 400];
    fake[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
    fake[398..].copy_from_slice(&[0xff, 0xd9]);
    fs::write(
        account.join("bad_t.dat"),
        encrypt(&fake, 123, "wxid_me", 64),
    )
    .unwrap();
    let result =
        acquire_image_keys(std::slice::from_ref(&dir), &account, Some("wxid_me"), 100).unwrap();
    assert_eq!(result["aes_verified"], true);
    assert_eq!(result["verified"], false);
    let mut wrong = encrypt(&jpeg(), 123, "wxid_me", 64);
    let n = wrong.len();
    for byte in &mut wrong[n - 64..] {
        *byte ^= 1;
    }
    fs::write(account.join("bad_t.dat"), wrong).unwrap();
    assert_eq!(
        acquire_image_keys(&[dir], &account, Some("wxid_me"), 100).unwrap()["verified"],
        false
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn acquisition_errors_are_distinct_and_multiple_pairs_are_ambiguous() {
    let root = common::temp_dir("image-key-errors");
    let dir = root.join("kvcomm");
    let account = root.join("wxid_me_suffix");
    let call = || acquire_image_keys(std::slice::from_ref(&dir), &account, Some("wxid_me"), 100);
    assert_eq!(call().unwrap_err().code, "image_key_directory_unreadable");
    fs::create_dir_all(&dir).unwrap();
    assert_eq!(call().unwrap_err().code, "image_key_no_candidates");
    fs::write(dir.join("key_123_suffix"), []).unwrap();
    assert_eq!(call().unwrap_err().code, "image_key_no_template");
    sample(&account, "first_t.dat", 456, "wxid_me", 64);
    assert_eq!(call().unwrap_err().code, "image_key_verification_failed");
    fs::write(dir.join("key_456_suffix"), []).unwrap();
    sample(&account, "second_t.dat", 123, "wxid_me", 64);
    let error = call().unwrap_err();
    assert_eq!(error.code, "image_key_ambiguous");
    assert!(!error
        .details
        .as_ref()
        .unwrap()
        .to_string()
        .contains("image_aes_key"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fully_verified_pair_outranks_an_aes_header_only_match() {
    let root = common::temp_dir("image-verified-preferred");
    let dir = root.join("kvcomm");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("key_123_456"), []).unwrap();
    let account = root.join("wxid_me_suffix");
    sample(&account, "full_t.dat", 123, "wxid_me", 64);
    sample(&account, "header_only_t.dat", 456, "wxid_me", 0);
    let result = acquire_image_keys(&[dir], &account, Some("wxid_me"), 100).unwrap();
    let (xor, aes) = derive_image_keys(123, "wxid_me");
    assert_eq!(result["image_aes_key"], aes);
    assert_eq!(result["image_xor_key"], xor);
    assert_eq!(result["verified"], true);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn templates_are_found_through_a_symlinked_account_root() {
    let root = common::temp_dir("image-symlink-root");
    let real = root.join("real");
    sample(&real, "a_t.dat", 123, "wxid_me", 64);
    let link = root.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(scan_templates(&link, 100, 32).valid_templates, 1);
    fs::remove_dir_all(root).unwrap();
}
