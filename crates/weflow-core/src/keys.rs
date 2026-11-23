//! Image-key derivation: the keys of a code, their check against a thumbnail, the thumbnail templates and the
//! candidate wxids. The `kvcomm` cache yields *codes*; the AES key depends on the account's wxid, so candidate wxids
//! are verified against a `_t.dat` template.

use std::path::{Path, PathBuf};

use aes::cipher::{Array, BlockCipherDecrypt, KeyInit};

const V2_MAGIC: [u8; 6] = [0x07, 0x08, 0x56, 0x32, 0x08, 0x07];

/// The wxid the image key is derived from: everything before the second underscore.
pub fn clean_wxid(wxid: &str) -> String {
    let Some(first) = wxid.find('_') else {
        return wxid.to_string();
    };
    match wxid[first + 1..].find('_') {
        Some(rel) => wxid[..first + 1 + rel].to_string(),
        None => wxid.to_string(),
    }
}

/// The image keys of a code: xor = code & 0xFF, aes = first 16 hex chars of md5(code + [`clean_wxid`]).
pub fn derive_image_keys(code: u64, wxid: &str) -> (u8, String) {
    use md5::{Digest, Md5};
    let hash: String = Md5::digest(format!("{code}{}", clean_wxid(wxid)).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    ((code & 0xff) as u8, hash[..16].to_string())
}

/// Whether an AES key is right: the key must decrypt the template block into an image header.
pub fn verify_derived_aes_key(aes_key: &str, ciphertext: &[u8]) -> bool {
    if !aes_key.is_ascii() {
        return false;
    }
    let (Some(key), Ok(ciphertext)) = (
        aes_key.as_bytes().first_chunk::<16>(),
        <&[u8; 16]>::try_from(ciphertext),
    ) else {
        return false;
    };
    let cipher = aes::Aes128::new(key.into());
    let mut block = Array::from(*ciphertext);
    cipher.decrypt_block(&mut block);
    let d = block.as_slice();
    // one block shows a RIFF file's start, not the `WEBP` at byte 8
    crate::image::ImageFormat::of(d).is_some() || d.starts_with(b"RIFF") || d.starts_with(b"wxgf")
}

/// Bounded template search: examine at most `budget` entries, then select the newest
/// structurally valid V2 thumbnails in that search, rather than stopping at `limit` files.
#[derive(Debug, Default, serde::Serialize)]
pub struct TemplateScan {
    /// The thumbnails kept, the newest first, at most `limit`.
    #[serde(skip)]
    pub templates: Vec<ImageTemplate>,
    /// The entries of the folder looked at.
    pub entries_scanned: usize,
    /// The `_t.dat` files in the V2 format with lengths that fit.
    pub valid_templates: usize,
    /// The `_t.dat` files of another format.
    pub invalid_format: usize,
    /// The V2 `_t.dat` files cut short or with lengths that do not fit.
    pub damaged_templates: usize,
    /// The entries that could not be read.
    pub read_errors: usize,
    /// Whether the folder has entries past the budget.
    pub truncated: bool,
    /// Whether more thumbnails were found than kept.
    pub templates_truncated: bool,
}

/// A V2 thumbnail to check image keys against.
#[derive(Debug)]
pub struct ImageTemplate {
    /// The `_t.dat` file.
    pub path: PathBuf,
    /// Its first AES block.
    pub cipher: [u8; 16],
    /// The XOR key its last two bytes give when they are a JPEG's end (`FF D9`); none when the XOR part is shorter.
    pub tail_xor: Option<u8>,
    /// When the file was last written.
    pub modified: Option<std::time::SystemTime>,
}

/// The V2 thumbnails (`_t.dat`) among the first `budget` entries of the account's folder `user_dir`, in name order,
/// the newest `limit` of them kept.
pub fn scan_templates(user_dir: &Path, budget: usize, limit: usize) -> TemplateScan {
    use std::io::{Read, Seek, SeekFrom};
    let mut scan = TemplateScan::default();
    let mut walk = walkdir::WalkDir::new(user_dir)
        .follow_links(false)
        .follow_root_links(true)
        .sort_by_file_name()
        .into_iter();
    for _ in 0..budget {
        let Some(entry) = walk.next() else {
            return finish_scan(scan, limit);
        };
        scan.entries_scanned += 1;
        let Ok(entry) = entry else {
            scan.read_errors += 1;
            continue;
        };
        if !entry.file_type().is_file() || !entry.file_name().to_string_lossy().ends_with("_t.dat")
        {
            continue;
        }
        let read = || -> std::io::Result<Option<ImageTemplate>> {
            let mut f = std::fs::File::open(entry.path())?;
            let metadata = f.metadata()?;
            let mut header = [0u8; 31];
            let mut magic = Vec::with_capacity(6);
            (&mut f).take(6).read_to_end(&mut magic)?;
            if magic != V2_MAGIC {
                return Ok(None);
            }
            header[..6].copy_from_slice(&magic);
            f.read_exact(&mut header[6..])?;
            let [_, _, _, _, _, _, a0, a1, a2, a3, x0, x1, x2, x3, _, cipher @ ..] = header;
            let aes_len = i32::from_le_bytes([a0, a1, a2, a3]);
            let xor_len = i32::from_le_bytes([x0, x1, x2, x3]);
            let aligned = (i64::from(aes_len) / 16 + 1) * 16;
            if aes_len < 16 || xor_len < 0 || metadata.len() < 15 + aligned as u64 + xor_len as u64
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid V2 lengths",
                ));
            }
            let tail_xor = if xor_len >= 2 {
                f.seek(SeekFrom::End(-2))?;
                let mut tail = [0; 2];
                f.read_exact(&mut tail)?;
                let k = tail[0] ^ 0xff;
                (k == tail[1] ^ 0xd9).then_some(k)
            } else {
                None
            };
            Ok(Some(ImageTemplate {
                path: entry.path().into(),
                cipher,
                tail_xor,
                modified: metadata.modified().ok(),
            }))
        };
        match read() {
            Ok(Some(t)) => {
                scan.valid_templates += 1;
                scan.templates.push(t);
            }
            Ok(None) => scan.invalid_format += 1,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::InvalidData
                ) =>
            {
                scan.damaged_templates += 1;
            }
            Err(_) => scan.read_errors += 1,
        }
    }
    scan.truncated = walk.next().is_some();
    finish_scan(scan, limit)
}

fn finish_scan(mut scan: TemplateScan, limit: usize) -> TemplateScan {
    scan.templates.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    scan.templates_truncated = scan.templates.len() > limit;
    scan.templates.truncate(limit);
    scan
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockCipherEncrypt;

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
        let cipher = aes::Aes128::new_from_slice(aes.as_bytes()).unwrap();
        let mut block = Array::from([
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
    fn a_scanned_template_yields_cipher_and_xor_key() {
        let dir = weflow_native::fixture::TestDir::new("weflow-keys");
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        let mut data = V2_MAGIC.to_vec();
        data.extend_from_slice(&16i32.to_le_bytes());
        data.extend_from_slice(&2i32.to_le_bytes());
        data.push(1);
        data.extend((0u8..16).collect::<Vec<_>>());
        data.extend_from_slice(&[0; 16]);
        data.extend_from_slice(&[0, 0]);
        // tail (x, y) with x ^ 0xFF == y ^ 0xD9
        let x = 0xa5u8;
        let y = (x ^ 0xff) ^ 0xd9;
        let n = data.len();
        data[n - 2] = x;
        data[n - 1] = y;
        std::fs::write(dir.join("a/b/x_t.dat"), &data).unwrap();
        std::fs::write(dir.join("a/other.dat"), b"nope").unwrap();
        let scan = scan_templates(&dir, 10_000, 32);
        let template = scan.templates.first().expect("the thumbnail is a template");
        assert_eq!(template.cipher[..4], [0, 1, 2, 3]);
        assert_eq!(template.tail_xor, Some(x ^ 0xff));
    }
}
