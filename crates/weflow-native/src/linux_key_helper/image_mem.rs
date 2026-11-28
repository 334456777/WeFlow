//! `image_mem` 模式：在微信可写的内存中搜索图片 AES 密钥。

use crate::image_key_scan::{find_key, Block, MemoryScanError, READ_OVERLAP};

use super::process;

const CHUNK: usize = 4 * 1024 * 1024;
const MAX_MAPPING: usize = 0x3200001;

/// 微信私有的可写映射中第一个能把 `ciphertexts` 之一解密成图片开头、且被 `accept` 接受的密钥（见
/// [`crate::image_key_scan`]）；没有这样的密钥时为 `Ok(None)`。
///
/// # Errors
///
/// 系统拒绝读取这个进程的任何内存时返回 [`MemoryScanError::PermissionDenied`]，没有 root 权限且
/// `/proc/sys/kernel/yama/ptrace_scope` 为 1 或更大时就是这样；读不了内存映射时返回 [`MemoryScanError::Failed`]。
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
                // 同样的拒绝适用于这个进程的每个地址
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
