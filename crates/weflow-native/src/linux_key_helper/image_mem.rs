//! The `image_mem` mode: search WeChat's writable memory for the image AES key.

use crate::image_key_scan::{find_key, Block, MemoryScanError, READ_OVERLAP};

use super::process;

const CHUNK: usize = 4 * 1024 * 1024;
const MAX_MAPPING: usize = 0x3200001;

/// The first key in WeChat's private writable mappings that decrypts one of `ciphertexts` into the start of an
/// image and that `accept` takes (see [`crate::image_key_scan`]); `Ok(None)` when no such key is there.
///
/// # Errors
///
/// [`MemoryScanError::PermissionDenied`] when the system refuses every read of the process's memory, as it does
/// without root while `/proc/sys/kernel/yama/ptrace_scope` is 1 or more; [`MemoryScanError::Failed`] when the
/// memory map cannot be read.
pub fn image_mem(
    pid: u32,
    ciphertexts: &[Block],
    accept: &mut dyn FnMut(&Block) -> bool,
) -> Result<Option<Block>, MemoryScanError> {
    let mappings = process::mappings(pid).map_err(|error| match error.kind() {
        std::io::ErrorKind::PermissionDenied => {
            MemoryScanError::PermissionDenied(error.to_string())
        }
        _ => MemoryScanError::Failed(error.to_string()),
    })?;
    let mut buffer = vec![0u8; CHUNK];
    let mut read_any = false;
    for mapping in mappings {
        if mapping.permissions != "rw-p" || mapping.end - mapping.start >= MAX_MAPPING {
            continue;
        }
        let mut start = mapping.start;
        while start < mapping.end {
            let length = (mapping.end - start).min(CHUNK);
            match process::read_memory_partial(pid, start, &mut buffer[..length]) {
                Ok(read) => {
                    read_any = true;
                    if let Some(key) = find_key(&buffer[..read], ciphertexts, accept) {
                        return Ok(Some(key));
                    }
                }
                // the same refusal applies to every address of the process
                Err(error) if !read_any && error.kind() == std::io::ErrorKind::PermissionDenied => {
                    return Err(MemoryScanError::PermissionDenied(format!(
                        "cannot read the memory of process {pid}: {error}"
                    )));
                }
                Err(_) => {}
            }
            if start + length >= mapping.end {
                break;
            }
            start += length - READ_OVERLAP;
        }
    }
    Ok(None)
}
