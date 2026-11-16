//! The `ScanMemoryForImageKey` export: find a 16-byte AES key among WeChat's
//! writable pages by decrypting the given ciphertext block with it and looking
//! for a JPEG header.

use aes::cipher::{Array, BlockCipherDecrypt, KeyInit};
use mach2::kern_return::KERN_SUCCESS;
use mach2::mach_port::mach_port_deallocate;
use mach2::port::mach_port_t;
use mach2::vm::mach_vm_region;
use mach2::vm_region::{vm_region_basic_info_64, VM_REGION_BASIC_INFO_64};
use mach2::vm_types::{integer_t, mach_vm_size_t};

use super::platform::{deallocate_task, read_memory, task_port, this_task};

const CHUNK: usize = 4 * 1024 * 1024;
const MAX_MAPPING: u64 = 0x320_0001;

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

fn decrypts_to_jpeg(key: &[u8; 16], ciphertext: &[u8; 16]) -> bool {
    let cipher = aes::Aes128::new(key.into());
    let mut block = Array::from(*ciphertext);
    cipher.decrypt_block(&mut block);
    block.as_slice().starts_with(&[0xff, 0xd8])
}

fn hex(key: &[u8]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The AES key as 32 hex characters, or `None` when no writable page holds it.
pub fn scan_memory_for_image_key(pid: u32, ciphertext: &str) -> Option<String> {
    let ciphertext = parse_ciphertext(ciphertext)?;
    let task = task_port(pid).ok()?;
    let result = scan(task, &ciphertext);
    deallocate_task(task);
    result
}

fn scan(task: mach2::mach_types::task_t, ciphertext: &[u8; 16]) -> Option<String> {
    let mut address = 0u64;
    loop {
        let mut size: mach_vm_size_t = 0;
        let mut object: mach_port_t = 0;
        let mut info = vm_region_basic_info_64::default();
        let mut count = vm_region_basic_info_64::count();
        // SAFETY: all out parameters are live locals of the types the flavor
        // writes; `address` starts at the region to query.
        let kr = unsafe {
            mach_vm_region(
                task,
                &mut address,
                &mut size,
                VM_REGION_BASIC_INFO_64,
                &mut info as *mut vm_region_basic_info_64 as *mut integer_t,
                &mut count,
                &mut object,
            )
        };
        if kr != KERN_SUCCESS || size == 0 {
            break;
        }
        if object != 0 {
            // SAFETY: the object right was returned by `mach_vm_region`.
            let _ = unsafe { mach_port_deallocate(this_task(), object) };
        }
        if info.protection & (libc::VM_PROT_READ | libc::VM_PROT_WRITE)
            == (libc::VM_PROT_READ | libc::VM_PROT_WRITE)
            && size < MAX_MAPPING
        {
            let mut offset = 0;
            while offset < size {
                let length = usize::try_from((size - offset).min(CHUNK as u64)).unwrap_or(CHUNK);
                if let Ok(read) = read_memory(task, address + offset, length) {
                    if read.len() >= 16 {
                        for window in read.windows(16) {
                            let key: &[u8; 16] = window.try_into().ok()?;
                            if decrypts_to_jpeg(key, ciphertext) {
                                return Some(hex(key));
                            }
                        }
                    }
                }
                offset += CHUNK as u64;
            }
        }
        address = address.saturating_add(size);
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
    fn finds_the_key_that_decrypts_to_a_jpeg_header() {
        let key = [0x42; 16];
        let mut block = Array::from(*b"\xff\xd8\xffhello world!!");
        aes::Aes128::new((&key).into()).encrypt_block(&mut block);
        let ciphertext: [u8; 16] = block.into();
        assert!(decrypts_to_jpeg(&key, &ciphertext));
        assert!(!decrypts_to_jpeg(&[0; 16], &ciphertext));
        assert_eq!(hex(&key), "42424242424242424242424242424242");
    }
}
