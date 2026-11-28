//! `key image-mem` 的内存扫描：在微信可写的页中找出 16 字节的图片 AES 密钥（见 [`crate::image_key_scan`]）。

use mach2::kern_return::KERN_SUCCESS;
use mach2::mach_port::mach_port_deallocate;
use mach2::port::mach_port_t;
use mach2::vm::mach_vm_region;
use mach2::vm_region::{vm_region_basic_info_64, VM_REGION_BASIC_INFO_64};
use mach2::vm_types::{integer_t, mach_vm_size_t};

use super::platform::{deallocate_task, read_memory, task_port, this_task};
use crate::image_key_scan::{find_key, Block, MemoryScanError, READ_OVERLAP};

const CHUNK: usize = 4 * 1024 * 1024;
const MAX_MAPPING: u64 = 0x320_0001;

/// 微信可读写的区域中第一个能把 `ciphertexts` 之一解密成图片开头、且被 `accept` 接受的密钥；
/// 没有这样的密钥时为 `Ok(None)`。
///
/// # Errors
///
/// 没有 root 权限等原因使 `task_for_pid` 拒绝给出微信的 task 端口时，
/// 返回 [`MemoryScanError::PermissionDenied`]。
pub fn scan_image_key(
    pid: u32,
    ciphertexts: &[Block],
    accept: &mut dyn FnMut(&Block) -> bool,
) -> Result<Option<Block>, MemoryScanError> {
    let task = task_port(pid).map_err(MemoryScanError::PermissionDenied)?;
    let result = scan(task, ciphertexts, accept);
    deallocate_task(task);
    Ok(result)
}

fn scan(
    task: mach2::mach_types::task_t,
    ciphertexts: &[Block],
    accept: &mut dyn FnMut(&Block) -> bool,
) -> Option<Block> {
    let mut address = 0u64;
    loop {
        let mut size: mach_vm_size_t = 0;
        let mut object: mach_port_t = 0;
        let mut info = vm_region_basic_info_64::default();
        let mut count = vm_region_basic_info_64::count();
        // SAFETY: 所有输出参数都是存活的局部变量，类型与这个查询类别（flavor）写入的一致；
        // `address` 是要查询的区域的起点。
        let kr = unsafe {
            mach_vm_region(
                task,
                &raw mut address,
                &raw mut size,
                VM_REGION_BASIC_INFO_64,
                (&raw mut info).cast::<integer_t>(),
                &raw mut count,
                &raw mut object,
            )
        };
        if kr != KERN_SUCCESS || size == 0 {
            break;
        }
        if object != 0 {
            // SAFETY: 这个对象的端口权限是 `mach_vm_region` 返回的。
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
                    if let Some(key) = find_key(&read, ciphertexts, accept) {
                        return Some(key);
                    }
                }
                if offset + length as u64 >= size {
                    break;
                }
                offset += (length - READ_OVERLAP) as u64;
            }
        }
        address = address.saturating_add(size);
    }
    None
}
