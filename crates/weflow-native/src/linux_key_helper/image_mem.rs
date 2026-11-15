//! The `image_mem` mode: scan writable memory for the image AES key.

use aes::cipher::{Array, BlockCipherDecrypt, KeyInit};

use super::process;

const CHUNK: usize = 4 * 1024 * 1024;
const MAX_MAPPING: usize = 0x3200001;

fn parse_ciphertext(value: &str) -> Option<[u8; 16]> {
    if value.len() != 32 {
        return None;
    }
    let mut ciphertext = [0u8; 16];
    for (index, byte) in ciphertext.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(ciphertext)
}

fn image_header(block: &[u8]) -> bool {
    block.starts_with(&[0xff, 0xd8, 0xff])
        || block.starts_with(b"\x89PNG")
        || block.starts_with(b"RIFF")
        || block.starts_with(b"wxgf")
        || block.starts_with(b"GIF")
}

fn decrypts_to_image(key: &[u8; 16], ciphertext: &[u8; 16]) -> bool {
    let cipher = aes::Aes128::new(key.into());
    let mut block = Array::from(*ciphertext);
    cipher.decrypt_block(&mut block);
    image_header(block.as_slice())
}

fn hex(key: &[u8]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Returns the 16-byte AES key as 32 hex characters, or `None`.
pub fn image_mem(pid: u32, ciphertext: &str) -> Option<String> {
    let ciphertext = parse_ciphertext(ciphertext)?;
    let mappings = process::mappings(pid).ok()?;
    let mut buffer = vec![0u8; CHUNK];
    for mapping in mappings {
        if mapping.permissions != "rw-p" || mapping.end - mapping.start >= MAX_MAPPING {
            continue;
        }
        let mut start = mapping.start;
        while start < mapping.end {
            let length = (mapping.end - start).min(CHUNK);
            if let Ok(read) = process::read_memory_partial(pid, start, &mut buffer[..length]) {
                if read > 16 {
                    for offset in 0..=read - 16 {
                        let key: &[u8; 16] = buffer[offset..offset + 16].try_into().ok()?;
                        if decrypts_to_image(key, &ciphertext) {
                            return Some(hex(key));
                        }
                    }
                }
            }
            start += length;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockCipherEncrypt;

    #[test]
    fn accepts_only_a_32_character_hex_ciphertext() {
        assert!(parse_ciphertext("00").is_none());
        assert!(parse_ciphertext(&"zz".repeat(16)).is_none());
        assert_eq!(parse_ciphertext(&"00".repeat(16)), Some([0; 16]));
    }

    #[test]
    fn recognizes_the_image_headers_the_helper_checks() {
        assert!(image_header(&[0xff, 0xd8, 0xff, 0]));
        assert!(image_header(b"\x89PNG"));
        assert!(image_header(b"RIFF"));
        assert!(image_header(b"wxgf"));
        assert!(image_header(b"GIF"));
        assert!(!image_header(b"\x89PNA"));
    }

    #[test]
    fn decrypts_the_known_cipher_block() {
        let key = [0x42; 16];
        let mut block = Array::from(*b"\xff\xd8\xffhello world!!");
        aes::Aes128::new((&key).into()).encrypt_block(&mut block);
        let ciphertext: [u8; 16] = block.into();
        assert!(decrypts_to_image(&key, &ciphertext));
    }
}
