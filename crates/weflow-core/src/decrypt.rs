use std::path::Path;

use aes::cipher::{BlockDecryptMut, KeyInit};
use anyhow::{anyhow, Result};
use md5::{Digest, Md5};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

type Aes128EcbDec = ecb::Decryptor<aes::Aes128>;

const V1_MAGIC: [u8; 6] = [0x07, 0x08, 0x56, 0x31, 0x08, 0x07];
const V2_MAGIC: [u8; 6] = [0x07, 0x08, 0x56, 0x32, 0x08, 0x07];

pub struct DecryptResult {
    pub data: Vec<u8>,
    pub ext: String,
    pub is_wxgf: bool,
}

pub fn detect_dat_version(data: &[u8]) -> u8 {
    if data.len() < 6 {
        return 0;
    }
    if data[..6] == V1_MAGIC {
        return 1;
    }
    if data[..6] == V2_MAGIC {
        return 2;
    }
    0
}

pub fn detect_image_extension(data: &[u8]) -> &str {
    if data.len() < 4 {
        return ".bin";
    }
    if data[..3] == [0xFF, 0xD8, 0xFF] {
        return ".jpg";
    }
    if data[..4] == [0x89, 0x50, 0x4E, 0x47] {
        return ".png";
    }
    if data.len() >= 12
        && data[..4] == [0x52, 0x49, 0x46, 0x46]
        && data[8..12] == [0x57, 0x45, 0x42, 0x50]
    {
        return ".webp";
    }
    if data[..3] == [0x47, 0x49, 0x46] {
        return ".gif";
    }
    if data.len() >= 4
        && data[..4] == [0x77, 0x78, 0x67, 0x66]
    {
        return ".wxgf";
    }
    ".bin"
}

/// `deriveImageKeys`: xor key is the low byte of the code, the AES key is the first 16 hex
/// characters of `md5(code + cleanedWxid)` used as ASCII text.
pub fn derive_image_keys(code: u64, wxid: &str) -> (u8, String) {
    let xor_key = (code & 0xFF) as u8;
    let cleaned_wxid = clean_wxid(wxid);
    let data_to_hash = format!("{}{}", code, cleaned_wxid);
    let mut hasher = Md5::new();
    hasher.update(data_to_hash.as_bytes());
    let digest = hasher.finalize();
    let full: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    (xor_key, full[..16].to_string())
}

/// Default key of V1 (`07 08 56 31 08 07`) `.dat` files.
pub const V1_AES_KEY: [u8; 16] = *b"cfcd208495d565ef";

/// Turns the configured image AES key into the 16 key bytes: the 16-character ASCII form the
/// desktop app stores (first 16 bytes), or a 32-digit hex string.
pub fn parse_aes_key(text: &str) -> Option<[u8; 16]> {
    let t = text.trim();
    if t.len() >= 32 && t.is_ascii() && t[..32].bytes().all(|b| b.is_ascii_hexdigit()) {
        let mut arr = [0u8; 16];
        for i in 0..16 {
            arr[i] = u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).ok()?;
        }
        return Some(arr);
    }
    if t.len() >= 16 && t.is_ascii() {
        let mut arr = [0u8; 16];
        arr.copy_from_slice(&t.as_bytes()[..16]);
        return Some(arr);
    }
    None
}

fn clean_wxid(wxid: &str) -> String {
    let trimmed = wxid.trim();
    if trimmed.to_lowercase().starts_with("wxid_") {
        if let Some(idx) = trimmed[5..].find('_') {
            return trimmed[..5 + idx].to_string();
        }
    }
    if let Some(idx) = trimmed.rfind('_') {
        let suffix = &trimmed[idx + 1..];
        if suffix.len() == 4 && suffix.chars().all(|c| c.is_ascii_alphanumeric()) {
            return trimmed[..idx].to_string();
        }
    }
    trimmed.to_string()
}

pub fn decrypt_dat(data: &[u8], xor_key: u8, aes_key: Option<&[u8; 16]>) -> Result<DecryptResult> {
    let version = detect_dat_version(data);
    match version {
        0 => {
            let ext = detect_image_extension(data).to_string();
            if ext != ".bin" {
                let is_wxgf = ext == ".wxgf";
                return Ok(DecryptResult { data: data.to_vec(), ext, is_wxgf });
            }
            // legacy V3: the whole file is XOR-ed with a single byte
            let xored: Vec<u8> = data.iter().map(|b| b ^ xor_key).collect();
            let ext = detect_image_extension(&xored).to_string();
            if ext != ".bin" {
                let is_wxgf = ext == ".wxgf";
                return Ok(DecryptResult { data: xored, ext, is_wxgf });
            }
            Ok(DecryptResult { data: data.to_vec(), ext, is_wxgf: false })
        }
        1 => decrypt_dat_v4(data, xor_key, &V1_AES_KEY),
        2 => {
            let aes = aes_key.ok_or_else(|| anyhow!("V2 .dat requires an AES key"))?;
            decrypt_dat_v4(data, xor_key, aes)
        }
        _ => Err(anyhow!("unknown .dat version: {version}")),
    }
}

/// `[15-byte header][AES-128-ECB part (PKCS7)][raw middle][XOR-ed tail]`; header holds the
/// plaintext size of the AES part at offset 6 and the tail size at offset 10.
fn decrypt_dat_v4(data: &[u8], xor_key: u8, aes_key: &[u8; 16]) -> Result<DecryptResult> {
    if data.len() < 0x0f {
        return Err(anyhow!("dat file too small"));
    }
    let payload = &data[0x0f..];
    let aes_size = read_i32_le(data, 6) as i64;
    let xor_size = read_i32_le(data, 10) as i64;
    let remainder = ((aes_size % 16) + 16) % 16;
    let aligned = aes_size + (16 - remainder);
    if aligned < 0 || aligned as usize > payload.len() {
        return Err(anyhow!("invalid aes size"));
    }
    let aligned = aligned as usize;
    let aes_data = &payload[..aligned];
    let plain_aes = if aes_data.is_empty() {
        Vec::new()
    } else {
        use aes::cipher::generic_array::GenericArray;
        let mut cipher = Aes128EcbDec::new(aes_key.into());
        let mut decrypted = Vec::with_capacity(aes_data.len());
        for chunk in aes_data.chunks(16) {
            let mut block = GenericArray::clone_from_slice(chunk);
            cipher.decrypt_block_mut(&mut block);
            decrypted.extend_from_slice(&block);
        }
        strict_remove_pkcs7(decrypted)?
    };
    let remaining = &payload[aligned..];
    if xor_size < 0 || xor_size as usize > remaining.len() {
        return Err(anyhow!("invalid xor size"));
    }
    let raw_len = remaining.len() - xor_size as usize;
    let mut out = Vec::with_capacity(plain_aes.len() + remaining.len());
    out.extend_from_slice(&plain_aes);
    out.extend_from_slice(&remaining[..raw_len]);
    out.extend(remaining[raw_len..].iter().map(|b| b ^ xor_key));
    let ext = detect_image_extension(&out).to_string();
    let is_wxgf = ext == ".wxgf";
    Ok(DecryptResult { data: out, ext, is_wxgf })
}

fn strict_remove_pkcs7(mut data: Vec<u8>) -> Result<Vec<u8>> {
    let pad = *data.last().ok_or_else(|| anyhow!("empty decrypted data"))? as usize;
    if pad == 0 || pad > 16 || pad > data.len() || data[data.len() - pad..].iter().any(|b| *b as usize != pad) {
        return Err(anyhow!("invalid pkcs7 padding"));
    }
    data.truncate(data.len() - pad);
    Ok(data)
}

fn read_i32_le(data: &[u8], offset: usize) -> i32 {
    let bytes = [
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ];
    i32::from_le_bytes(bytes)
}

pub fn decrypt_file(
    path: &Path,
    xor_key: u8,
    aes_key: Option<&[u8; 16]>,
) -> AppResult<DecryptResult> {
    let data = std::fs::read(path).map_err(|err| {
        AppError::runtime(format!("failed to read {}: {err}", path.display()))
    })?;
    decrypt_dat(&data, xor_key, aes_key).map_err(|err| AppError::runtime(err.to_string()))
}

pub fn decrypt_file_to_json(path: &Path, xor_key: u8, aes_key: Option<&[u8; 16]>) -> AppResult<Value> {
    let result = decrypt_file(path, xor_key, aes_key)?;
    Ok(json!({
        "path": path.to_string_lossy(),
        "ext": result.ext,
        "isWxgf": result.is_wxgf,
        "size": result.data.len()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_v1_magic() {
        let mut data = vec![0u8; 32];
        data[..6].copy_from_slice(&V1_MAGIC);
        assert_eq!(detect_dat_version(&data), 1);
    }

    #[test]
    fn detects_v2_magic() {
        let mut data = vec![0u8; 32];
        data[..6].copy_from_slice(&V2_MAGIC);
        assert_eq!(detect_dat_version(&data), 2);
    }

    #[test]
    fn detects_raw_version() {
        let data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(detect_dat_version(&data), 0);
    }

    #[test]
    fn detects_jpeg() {
        assert_eq!(detect_image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), ".jpg");
    }

    #[test]
    fn detects_png() {
        assert_eq!(
            detect_image_extension(&[0x89, 0x50, 0x4E, 0x47]),
            ".png"
        );
    }

    #[test]
    fn detects_webp() {
        let data = [0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50];
        assert_eq!(detect_image_extension(&data), ".webp");
    }

    #[test]
    fn detects_gif() {
        assert_eq!(detect_image_extension(&[0x47, 0x49, 0x46, 0x38]), ".gif");
    }

    /// Builds a V4-layout `.dat` the way WeChat writes it.
    fn encrypt_v4(sig: [u8; 6], plain: &[u8], aes_len: usize, xor_len: usize, key: &[u8; 16], xor_key: u8) -> Vec<u8> {
        use aes::cipher::{generic_array::GenericArray, BlockEncryptMut, KeyInit};
        let aes_part = &plain[..aes_len];
        let xor_part = &plain[plain.len() - xor_len..];
        let raw_part = &plain[aes_len..plain.len() - xor_len];
        let pad = 16 - aes_part.len() % 16;
        let mut padded = aes_part.to_vec();
        padded.extend(std::iter::repeat(pad as u8).take(pad));
        let mut enc = ecb::Encryptor::<aes::Aes128>::new(key.into());
        let mut cipher = Vec::new();
        for chunk in padded.chunks(16) {
            let mut block = GenericArray::clone_from_slice(chunk);
            enc.encrypt_block_mut(&mut block);
            cipher.extend_from_slice(&block);
        }
        let mut out = sig.to_vec();
        out.extend_from_slice(&(aes_len as i32).to_le_bytes());
        out.extend_from_slice(&(xor_len as i32).to_le_bytes());
        out.push(0x01);
        out.extend(cipher);
        out.extend_from_slice(raw_part);
        out.extend(xor_part.iter().map(|b| b ^ xor_key));
        out
    }

    fn sample_jpeg(len: usize) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
        v.extend((0..len - 4).map(|i| (i % 251) as u8));
        v
    }

    #[test]
    fn decrypts_v2_with_raw_middle_and_xor_tail() {
        let key = *b"0123456789abcdef";
        let plain = sample_jpeg(3000);
        for aes_len in [1024usize, 1000, 16] {
            let file = encrypt_v4(V2_MAGIC, &plain, aes_len, 200, &key, 0x5a);
            let got = decrypt_dat(&file, 0x5a, Some(&key)).unwrap();
            assert_eq!(got.data, plain, "aes_len={aes_len}");
            assert_eq!(got.ext, ".jpg");
        }
        assert!(decrypt_dat(&encrypt_v4(V2_MAGIC, &plain, 1024, 0, &key, 1), 1, None).is_err(), "V2 needs a key");
        let wrong = decrypt_dat(&encrypt_v4(V2_MAGIC, &plain, 1024, 0, &key, 1), 1, Some(b"ffffffffffffffff"));
        assert!(wrong.is_err(), "bad key fails the strict padding check");
    }

    #[test]
    fn decrypts_v1_with_the_default_key_and_legacy_xor() {
        let plain = sample_jpeg(2048);
        let file = encrypt_v4(V1_MAGIC, &plain, 1024, 64, &V1_AES_KEY, 0x11);
        assert_eq!(decrypt_dat(&file, 0x11, None).unwrap().data, plain);
        // no signature: already an image, or a whole-file XOR
        assert_eq!(decrypt_dat(&plain, 0x11, None).unwrap().data, plain);
        let xored: Vec<u8> = plain.iter().map(|b| b ^ 0x11).collect();
        let got = decrypt_dat(&xored, 0x11, None).unwrap();
        assert_eq!(got.data, plain);
        assert_eq!(got.ext, ".jpg");
    }

    #[test]
    fn aes_keys_parse_from_ascii_or_hex() {
        assert_eq!(parse_aes_key("0123456789abcdef"), Some(*b"0123456789abcdef"));
        assert_eq!(parse_aes_key(" 0123456789abcdefXYZ "), Some(*b"0123456789abcdef"));
        let hex = "00112233445566778899aabbccddeeff";
        assert_eq!(parse_aes_key(hex).unwrap()[15], 0xff);
        assert_eq!(parse_aes_key("short"), None);
    }

    #[test]
    fn derives_image_keys_deterministically() {
        let (xor1, aes1) = derive_image_keys(12345, "wxid_test");
        let (xor2, aes2) = derive_image_keys(12345, "wxid_test");
        assert_eq!(xor1, xor2);
        assert_eq!(aes1, aes2);
        assert_eq!(xor1, (12345u64 & 0xFF) as u8);
        assert_eq!(aes1.len(), 16);
        assert!(aes1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn clean_wxid_strips_suffix() {
        assert_eq!(clean_wxid("wxid_abc_1234"), "wxid_abc");
        assert_eq!(clean_wxid("wxid_abc"), "wxid_abc");
    }

    #[test]
    fn read_i32_le_correct() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(read_i32_le(&data, 6), 7);
        assert_eq!(read_i32_le(&data, 10), 8);
    }
}
