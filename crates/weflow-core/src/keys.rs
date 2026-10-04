//! Image-key derivation of `keyService.ts` (`deriveImageKeys`, `verifyDerivedAesKey`,
//! `_findTemplateData`, `collectWxidCandidates`). The `kvcomm` cache yields *codes*; the AES key
//! depends on the account's wxid, so candidate wxids are verified against a `_t.dat` template.
use std::path::{Path, PathBuf};

use aes::cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};

const V2_MAGIC: [u8; 6] = [0x07, 0x08, 0x56, 0x32, 0x08, 0x07];

/// `cleanWxid` of the key service: keeps everything before the second underscore.
pub fn clean_wxid(wxid: &str) -> String {
    let Some(first) = wxid.find('_') else {
        return wxid.to_string();
    };
    match wxid[first + 1..].find('_') {
        Some(rel) => wxid[..first + 1 + rel].to_string(),
        None => wxid.to_string(),
    }
}

/// `deriveImageKeys`: xor = code & 0xFF, aes = first 16 hex chars of md5(code + cleanedWxid).
pub fn derive_image_keys(code: u64, wxid: &str) -> (u8, String) {
    use md5::{Digest, Md5};
    let hash: String = Md5::digest(format!("{code}{}", clean_wxid(wxid)).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    ((code & 0xff) as u8, hash[..16].to_string())
}

/// `verifyDerivedAesKey`: the key must decrypt the template block into an image header.
pub fn verify_derived_aes_key(aes_key: &str, ciphertext: &[u8]) -> bool {
    if aes_key.len() < 16 || ciphertext.len() != 16 || !aes_key.is_ascii() {
        return false;
    }
    let cipher = aes::Aes128::new(GenericArray::from_slice(&aes_key.as_bytes()[..16]));
    let mut block = GenericArray::clone_from_slice(ciphertext);
    cipher.decrypt_block(&mut block);
    let d = block.as_slice();
    (d[0] == 0xff && d[1] == 0xd8 && d[2] == 0xff)
        || d[..4] == [0x89, 0x50, 0x4e, 0x47]
        || d[..4] == [0x52, 0x49, 0x46, 0x46]
        || d[..4] == [0x77, 0x78, 0x67, 0x66]
        || d[..3] == [0x47, 0x49, 0x46]
}

fn collect_t_dat(dir: &Path, out: &mut Vec<PathBuf>, max: usize) {
    if out.len() >= max {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.filter_map(Result::ok) {
        if out.len() >= max {
            break;
        }
        let p = e.path();
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            collect_t_dat(&p, out, max);
        } else if t.is_file()
            && p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with("_t.dat"))
        {
            out.push(p);
        }
    }
}

/// `_findTemplateData`: ciphertext block of the first V2 `_t.dat` (newest first) and the XOR key
/// derived from the most common tail bytes (`x ^ 0xFF == y ^ 0xD9`).
pub fn find_template_data(user_dir: &Path, limit: usize) -> (Option<[u8; 16]>, Option<u8>) {
    let mut files = Vec::new();
    collect_t_dat(user_dir, &mut files, limit);
    files.sort_by(|a, b| {
        let m = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        m(b).cmp(&m(a))
    });
    let mut cipher: Option<[u8; 16]> = None;
    let mut tails: Vec<((u8, u8), usize)> = Vec::new();
    for f in files.iter().take(32) {
        let Ok(data) = std::fs::read(f) else { continue };
        if data.len() < 8 || data[..6] != V2_MAGIC {
            continue;
        }
        let key = (data[data.len() - 2], data[data.len() - 1]);
        match tails.iter_mut().find(|t| t.0 == key) {
            Some(t) => t.1 += 1,
            None => tails.push((key, 1)),
        }
        if cipher.is_none() && data.len() >= 0x1f {
            let mut c = [0u8; 16];
            c.copy_from_slice(&data[0xf..0x1f]);
            cipher = Some(c);
        }
    }
    let mut xor = None;
    let mut max = 0;
    for ((x, y), count) in tails {
        if count > max {
            max = count;
            let k = x ^ 0xff;
            if k == (y ^ 0xd9) {
                xor = Some(k);
            }
        }
    }
    (cipher, xor)
}

/// `collectWxidCandidates`: the configured wxid, the directory name and sibling account
/// directories under `xwechat_files` / `WeChat Files`, and finally `unknown`.
pub fn collect_wxid_candidates(manual_dir: Option<&str>, wxid: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |v: &str| {
        let v = v.trim();
        if !v.is_empty() && !out.iter().any(|o| o == v) {
            out.push(v.to_string());
        }
    };
    if let Some(w) = wxid.filter(|w| w.starts_with("wxid_")) {
        push(w);
    }
    if let Some(dir) = manual_dir {
        let norm = dir.trim_end_matches(['\\', '/']);
        let name = norm.rsplit(['\\', '/']).next().unwrap_or("");
        if name.starts_with("wxid_") {
            push(name);
        }
        let lower = norm.to_lowercase();
        for marker in ["xwechat_files", "wechat files"] {
            if let Some(pos) = lower.find(marker) {
                let root = &norm[..pos + marker.len()];
                if let Ok(rd) = std::fs::read_dir(root) {
                    let mut names: Vec<String> = rd
                        .filter_map(Result::ok)
                        .filter(|e| e.path().is_dir())
                        .map(|e| e.file_name().to_string_lossy().to_string())
                        .filter(|n| n.starts_with("wxid_"))
                        .collect();
                    names.sort();
                    for n in names {
                        push(&n);
                    }
                }
                break;
            }
        }
    }
    push("unknown");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncrypt;

    #[test]
    fn wxid_is_cut_at_the_second_underscore() {
        assert_eq!(clean_wxid("wxid_abc123_e50d"), "wxid_abc123");
        assert_eq!(clean_wxid("wxid_abc123"), "wxid_abc123");
        assert_eq!(clean_wxid("plain"), "plain");
        assert_eq!(
            clean_wxid("name_1234"),
            "name_1234",
            "only wxid-style names with two underscores are cut"
        );
    }

    #[test]
    fn derivation_and_verification_round_trip() {
        let (xor, aes) = derive_image_keys(0x1234_5a5a, "wxid_abc_e50d");
        assert_eq!(xor, 0x5a);
        assert_eq!(aes.len(), 16);
        assert_eq!(
            (xor, aes.clone()),
            derive_image_keys(0x1234_5a5a, "wxid_abc")
        );
        let cipher = aes::Aes128::new(GenericArray::from_slice(aes.as_bytes()));
        let mut block = GenericArray::clone_from_slice(&[
            0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
        ]);
        cipher.encrypt_block(&mut block);
        assert!(verify_derived_aes_key(&aes, block.as_slice()));
        assert!(!verify_derived_aes_key(
            "0123456789abcdef",
            block.as_slice()
        ));
        assert!(!verify_derived_aes_key("short", block.as_slice()));
    }

    #[test]
    fn template_data_yields_cipher_and_xor_key() {
        let dir = std::env::temp_dir().join(format!("weflow-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        let mut data = V2_MAGIC.to_vec();
        data.extend_from_slice(&[0; 9]);
        data.extend((0u8..16).collect::<Vec<_>>());
        data.extend_from_slice(&[0x40, 0x40 ^ 0xff ^ 0xd9 ^ 0xff ^ 0xff]);
        // tail (x, y) with x ^ 0xFF == y ^ 0xD9
        let x = 0xa5u8;
        let y = (x ^ 0xff) ^ 0xd9;
        let n = data.len();
        data[n - 2] = x;
        data[n - 1] = y;
        std::fs::write(dir.join("a/b/x_t.dat"), &data).unwrap();
        std::fs::write(dir.join("a/other.dat"), b"nope").unwrap();
        let (c, k) = find_template_data(&dir, 32);
        assert_eq!(c.unwrap()[..4], [0, 1, 2, 3]);
        assert_eq!(k, Some(x ^ 0xff));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn candidates_include_siblings() {
        let root =
            std::env::temp_dir().join(format!("weflow-cand-{}/xwechat_files", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("wxid_a_1111")).unwrap();
        std::fs::create_dir_all(root.join("wxid_b_2222")).unwrap();
        std::fs::create_dir_all(root.join("all_users")).unwrap();
        let c = collect_wxid_candidates(
            Some(root.join("wxid_b_2222").to_str().unwrap()),
            Some("wxid_cfg"),
        );
        assert_eq!(c, vec!["wxid_cfg", "wxid_b_2222", "wxid_a_1111", "unknown"]);
        let _ = std::fs::remove_dir_all(root.parent().unwrap());
    }
}
