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

pub fn derive_image_keys(code: u64, wxid: &str) -> (u8, String) {
    let xor_key = (code & 0xFF) as u8;
    let cleaned_wxid = clean_wxid(wxid);
    let data_to_hash = format!("{}{}", code, cleaned_wxid);
    let mut hasher = Md5::new();
    hasher.update(data_to_hash.as_bytes());
    let digest = hasher.finalize();
    let aes_key = format!("{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3],
        digest[4], digest[5], digest[6], digest[7],
        digest[8], digest[9], digest[10], digest[11],
        digest[12], digest[13], digest[14], digest[15]
    );
    (xor_key, aes_key)
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
            Ok(DecryptResult {
                data: data.to_vec(),
                ext,
                is_wxgf: false,
            })
        }
        1 => decrypt_dat_v1(data, xor_key),
        2 => {
            let aes = aes_key.ok_or_else(|| anyhow!("V2 .dat requires an AES key"))?;
            decrypt_dat_v2(data, xor_key, aes)
        }
        _ => Err(anyhow!("unknown .dat version: {version}")),
    }
}

fn decrypt_dat_v1(data: &[u8], xor_key: u8) -> Result<DecryptResult> {
    // V1: magic bytes at [0..6], payload from [6..] is XOR'd
    let decrypted: Vec<u8> = data[6..].iter().map(|b| b ^ xor_key).collect();
    let ext = detect_image_extension(&decrypted).to_string();
    let is_wxgf = ext == ".wxgf";
    Ok(DecryptResult { data: decrypted, ext, is_wxgf })
}

fn decrypt_dat_v2(data: &[u8], xor_key: u8, aes_key: &[u8; 16]) -> Result<DecryptResult> {
    if data.len() < 0x0f {
        return Err(anyhow!(".dat file too small for V2"));
    }
    let header = &data[..0x0f];
    let payload = &data[0x0f..];

    let aes_size = read_i32_le(header, 6);
    let _xor_size = read_i32_le(header, 10);

    let remainder = ((aes_size % 16) + 16) % 16;
    let aligned_aes_size = if remainder == 0 {
        aes_size as usize
    } else {
        (aes_size as usize) + (16 - remainder as usize)
    };
    if aligned_aes_size > payload.len() {
        return Err(anyhow!("invalid AES size in .dat header"));
    }

    let aes_data = &payload[..aligned_aes_size];
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
        strip_pkcs7(&decrypted, aes_size as usize)
    };

    let xor_data = &payload[aligned_aes_size..];
    let decoded_xor: Vec<u8> = xor_data.iter().map(|b| b ^ xor_key).collect();

    let mut result = Vec::with_capacity(plain_aes.len() + decoded_xor.len());
    result.extend_from_slice(&plain_aes);
    result.extend_from_slice(&decoded_xor);

    let ext = detect_image_extension(&result).to_string();
    let is_wxgf = ext == ".wxgf";
    Ok(DecryptResult { data: result, ext, is_wxgf })
}

fn strip_pkcs7(data: &[u8], expected_size: usize) -> Vec<u8> {
    if data.len() < expected_size {
        return data.to_vec();
    }
    data[..expected_size].to_vec()
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

    #[test]
    fn decrypts_v1_xor() {
        let xor_key = 0xAB;
        let original_data = vec![0xFF, 0xD8, 0xFF, 0xE0]; // JPEG header
        let mut file = Vec::new();
        file.extend_from_slice(&V1_MAGIC);
        for b in &original_data {
            file.push(b ^ xor_key);
        }
        let result = decrypt_dat(&file, xor_key, None).unwrap();
        assert_eq!(&result.data[..4], &original_data[..]);
    }

    #[test]
    fn derives_image_keys_deterministically() {
        let (xor1, aes1) = derive_image_keys(12345, "wxid_test");
        let (xor2, aes2) = derive_image_keys(12345, "wxid_test");
        assert_eq!(xor1, xor2);
        assert_eq!(aes1, aes2);
        assert_eq!(xor1, (12345u64 & 0xFF) as u8);
        assert_eq!(aes1.len(), 32);
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
