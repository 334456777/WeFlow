//! The memory scan of `key image-mem`: find the 16-byte image AES key among WeChat's writable pages (see
//! [`crate::image_key_scan`]).

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

/// The first key in WeChat's readable and writable regions that decrypts one of `ciphertexts` into the start of an
/// image and that `accept` takes; `Ok(None)` when no such key is there.
///
/// # Errors
///
/// [`MemoryScanError::PermissionDenied`] when `task_for_pid` refuses WeChat's task port, as it does without root.
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
