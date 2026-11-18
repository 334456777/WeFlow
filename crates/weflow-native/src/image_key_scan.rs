//! Finding the image AES key among the bytes of WeChat's memory.
//!
//! Every 16-byte window is tried as a key on the first encrypted block of a few thumbnails, and a window that turns
//! one of them into the start of an image is a candidate. The memory readers of `linux_key_helper` and
//! `macos_key_helper` share this search.
//!
//! WeChat's memory holds around 10^9 windows, so a check of the first few bytes alone lets random windows through:
//! [`image_block`] checks the structure of the whole block, and the caller confirms each candidate before taking it.

use aes::cipher::{Array, BlockCipherDecrypt, KeyInit};

/// One AES block, the size of the image AES key and of the ciphertext the key is searched with.
pub type Block = [u8; 16];

/// The PNG signature followed by the length and type of the `IHDR` chunk every PNG starts with.
const PNG_START: Block = *b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR";

/// Why WeChat's memory could not be searched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryScanError {
    /// The system refused to let this process read WeChat's memory (no root, `ptrace_scope`, `task_for_pid`).
    PermissionDenied(String),
    /// The process's memory map could not be read for another reason.
    Failed(String),
}

impl std::fmt::Display for MemoryScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionDenied(detail) => {
                write!(f, "permission denied reading WeChat's memory: {detail}")
            }
            Self::Failed(detail) => write!(f, "cannot read WeChat's memory: {detail}"),
        }
    }
}

impl std::error::Error for MemoryScanError {}

/// Whether a decrypted first block is the start of an image file WeChat stores.
///
/// That is a JPEG whose first segment is an `APPn` segment with an ASCII identifier (`JFIF`, `Exif`, ...) or a
/// quantization table, a PNG up to its `IHDR` chunk type, a GIF with its version, a WebP with its `RIFF`/`WEBP`
/// header, or WeChat's own `wxgf` container.
#[must_use]
pub fn image_block(block: &Block) -> bool {
    jpeg_start(block)
        || *block == PNG_START
        || block.starts_with(b"GIF87a")
        || block.starts_with(b"GIF89a")
        || (block.starts_with(b"RIFF") && block[8..12] == *b"WEBP")
        || block.starts_with(b"wxgf")
}

/// A JPEG's start of image followed by the first segment's marker, length and the start of its data.
fn jpeg_start(block: &Block) -> bool {
    let [0xff, 0xd8, 0xff, marker, length_high, _, data @ ..] = *block else {
        return false;
    };
    match marker {
        // APP0..APP15 begin with a NUL-terminated identifier such as "JFIF" or "Exif"
        0xe0..=0xef => data[..4].iter().all(u8::is_ascii_alphanumeric),
        // DQT: 67 bytes per 8-bit table, at most four tables; precision 0 or 1 and table id 0..=3
        0xdb => length_high <= 1 && data[0] & 0xec == 0,
        _ => false,
    }
}

/// Whether `key` decrypts `ciphertext` into an [`image_block`].
#[must_use]
pub fn decrypts_to_image(key: &Block, ciphertext: &Block) -> bool {
    let cipher = aes::Aes128::new(key.into());
    let mut block = Array::from(*ciphertext);
    cipher.decrypt_block(&mut block);
    image_block(&block.into())
}

/// The 16 bytes a ciphertext of 32 hex characters names, or `None` for anything else.
#[must_use]
pub fn parse_ciphertext(value: &str) -> Option<Block> {
    if value.len() != 32 || !value.is_ascii() {
        return None;
    }
    let mut ciphertext = [0u8; 16];
    for (index, byte) in ciphertext.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(ciphertext)
}

/// `key` as 32 lowercase hex characters.
#[must_use]
pub fn hex(key: &Block) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// How far consecutive reads of one region overlap, so a key that straddles two reads is still seen whole.
#[cfg(any(all(target_os = "linux", target_arch = "x86_64"), target_os = "macos"))]
pub(crate) const READ_OVERLAP: usize = 15;

/// The first 16-byte window of `memory` that decrypts one of `ciphertexts` into an [`image_block`] and that `accept`
/// takes; `accept` sees every such window, in order, until it takes one.
#[cfg(any(
    test,
    all(target_os = "linux", target_arch = "x86_64"),
    target_os = "macos"
))]
pub(crate) fn find_key(
    memory: &[u8],
    ciphertexts: &[Block],
    accept: &mut dyn FnMut(&Block) -> bool,
) -> Option<Block> {
    for window in memory.windows(16) {
        let Ok(key) = <&Block>::try_from(window) else {
            continue;
        };
        let cipher = aes::Aes128::new(key.into());
        let candidate = ciphertexts.iter().any(|ciphertext| {
            let mut block = Array::from(*ciphertext);
            cipher.decrypt_block(&mut block);
            image_block(&block.into())
        });
        if candidate && accept(key) {
            return Some(*key);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockCipherEncrypt;

    const JFIF: Block = *b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01";

    fn encrypt(key: &Block, plain: &Block) -> Block {
        let mut block = Array::from(*plain);
        aes::Aes128::new(key.into()).encrypt_block(&mut block);
        block.into()
    }

    #[test]
    fn accepts_only_a_32_character_hex_ciphertext() {
        assert!(parse_ciphertext("00").is_none());
        assert!(parse_ciphertext(&"zz".repeat(16)).is_none());
        assert!(parse_ciphertext(&"é".repeat(16)).is_none());
        assert_eq!(parse_ciphertext(&"00".repeat(16)), Some([0; 16]));
        assert_eq!(hex(&[0x42; 16]), "42".repeat(16));
    }

    #[test]
    fn image_blocks_are_checked_beyond_their_magic_bytes() {
        let exif = *b"\xff\xd8\xff\xe1\x12\x34Exif\x00\x00MM\x00\x2a";
        let dqt = *b"\xff\xd8\xff\xdb\x00\x43\x00\x08\x06\x06\x07\x06\x05\x08\x07\x07";
        let webp = *b"RIFF\x10\x20\x00\x00WEBPVP8 ";
        for block in [
            JFIF,
            exif,
            dqt,
            PNG_START,
            *b"GIF89a\x10\x00\x10\x00\xf7\x00\x00\x00\x00\x00",
            webp,
            *b"wxgf\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c",
        ] {
            assert!(image_block(&block), "{block:02x?}");
        }
        // the magic bytes of a JPEG, PNG or GIF followed by bytes no such file starts with
        for block in [
            *b"\xff\xd8\xffhello world!!",
            *b"\xff\xd8\xff\xe0\x00\x10\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09",
            *b"\xff\xd8\xff\xdb\x07\x43\x00\x08\x06\x06\x07\x06\x05\x08\x07\x07",
            *b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDX",
            *b"GIF\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d",
            *b"RIFF\x10\x20\x00\x00WAVEfmt ",
        ] {
            assert!(!image_block(&block), "{block:02x?}");
        }
    }

    #[test]
    fn finds_the_window_that_decrypts_a_ciphertext_and_is_accepted() {
        let key = [0x42; 16];
        let other = [0x17; 16];
        let ciphertexts = [encrypt(&[0x99; 16], &JFIF), encrypt(&key, &JFIF)];
        let mut memory = vec![0u8; 40];
        memory.extend_from_slice(&other);
        memory.extend_from_slice(&[0u8; 7]);
        memory.extend_from_slice(&key);
        memory.extend_from_slice(&[0u8; 3]);
        let mut seen = Vec::new();
        let found = find_key(&memory, &ciphertexts, &mut |candidate| {
            seen.push(*candidate);
            true
        });
        assert_eq!(found, Some(key));
        assert_eq!(
            seen,
            [key],
            "only a window that decrypts a ciphertext is offered"
        );
        // a refused candidate does not end the search, nor is it returned
        assert_eq!(find_key(&memory, &ciphertexts, &mut |_| false), None);
        assert!(decrypts_to_image(&key, &ciphertexts[1]));
        assert!(!decrypts_to_image(&other, &ciphertexts[1]));
    }
}
