//! 在微信进程的内存中找出图片 AES 密钥。
//!
//! 把每个 16 字节的窗口当作密钥，去解密几张缩略图的第一个加密块，能把其中一张解密成图片开头的窗口就是候选。
//! `linux_key_helper` 和 `macos_key_helper` 的内存读取共用这个搜索。
//!
//! 微信的内存中约有 10^9 个窗口，只检查开头几个字节会放过随机的窗口：[`image_block`] 检查整个块的结构，
//! 调用方在采用每个候选之前还会再确认。

use aes::cipher::{Array, BlockCipherDecrypt, KeyInit};

/// 一个 AES 块，与图片 AES 密钥和搜索密钥时用的密文一样长。
pub type Block = [u8; 16];

/// PNG 签名，后接每个 PNG 开头的 `IHDR` 块的长度和类型。
const PNG_START: Block = *b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR";

/// 无法搜索微信内存的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryScanError {
    /// 系统不允许本进程读取微信的内存（没有 root 权限、`ptrace_scope` 的限制、`task_for_pid` 失败）。
    PermissionDenied(String),
    /// 因其他原因读不了进程的内存映射。
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

/// 解密出的第一个块是否为微信保存的某种图片文件的开头。
///
/// 包括：第一个段是带 ASCII 标识（`JFIF`、`Exif` 等）的 `APPn` 段或量化表的 JPEG，直到 `IHDR` 块类型为止的 PNG，
/// 带版本号的 GIF，带 `RIFF`/`WEBP` 头的 WebP，以及微信私有的 `wxgf` 容器。
#[must_use]
pub fn image_block(block: &Block) -> bool {
    jpeg_start(block)
        || *block == PNG_START
        || block.starts_with(b"GIF87a")
        || block.starts_with(b"GIF89a")
        || (block.starts_with(b"RIFF") && block[8..12] == *b"WEBP")
        || block.starts_with(b"wxgf")
}

/// JPEG 的图像开始标记，后接第一个段的标记、长度和数据的开头。
fn jpeg_start(block: &Block) -> bool {
    let [0xff, 0xd8, 0xff, marker, length_high, _, data @ ..] = *block else {
        return false;
    };
    match marker {
        // APP0..APP15 以 NUL 结尾的标识开头，如 `JFIF`、`Exif`
        0xe0..=0xef => data[..4].iter().all(u8::is_ascii_alphanumeric),
        // DQT：每张 8 位的表 67 字节，最多四张表；精度为 0 或 1，表 ID 为 0..=3
        0xdb => length_high <= 1 && data[0] & 0xec == 0,
        _ => false,
    }
}

/// `key` 能否把 `ciphertext` 解密成 [`image_block`]。
#[must_use]
pub fn decrypts_to_image(key: &Block, ciphertext: &Block) -> bool {
    let cipher = aes::Aes128::new(key.into());
    let mut block = Array::from(*ciphertext);
    cipher.decrypt_block(&mut block);
    image_block(&block.into())
}

/// 32 个十六进制字符表示的密文的 16 个字节；其他输入为 `None`。
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

/// 把 `key` 写成 32 个小写十六进制字符。
#[must_use]
pub fn hex(key: &Block) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 同一区域相邻两次读取重叠的字节数，跨两次读取的密钥仍能完整看到。
#[cfg(any(all(target_os = "linux", target_arch = "x86_64"), target_os = "macos"))]
pub(crate) const READ_OVERLAP: usize = 15;

/// `memory` 中第一个能把 `ciphertexts` 之一解密成 [`image_block`]、且被 `accept` 接受的 16 字节窗口；
/// `accept` 按顺序看到每个这样的窗口，直到接受其中一个。
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
        // JPEG、PNG 或 GIF 的魔数，后面跟着这些文件不会有的开头
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
        // 被拒绝的候选既不会结束搜索，也不会被返回
        assert_eq!(find_key(&memory, &ciphertexts, &mut |_| false), None);
        assert!(decrypts_to_image(&key, &ciphertexts[1]));
        assert!(!decrypts_to_image(&other, &ciphertexts[1]));
    }
}
