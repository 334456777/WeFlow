//! The machine code `img_helper.dll` writes into `Weixin.dll` to make WeChat download original-size images.
//!
//! The hook replaces the first 13 bytes of the target function with `mov r10, <shellcode>; jmp r10`. The shellcode
//! saves the registers, checks whether the download's clientid (the second argument's `+0x50` field, which WeChat
//! builds as `<conversation id>_<timestamp>_<number>`) starts with one of the whitelisted conversation ids followed
//! by `_`, and when it does, changes the download type at `+0xd0` from 3 to 2. It then runs the 13 bytes it replaced
//! and jumps back to the instruction after them.
//!
//! The relative jumps are fixed: the shellcode is always laid out the same way, with the whitelist at `+0x300` and
//! the original bytes at `+0xb4`. Only the original bytes and the address they return to are filled in at run time.

/// The shellcode up to the saved registers, with every relative jump already resolved.
///
/// The whitelist is a NUL-separated, double-NUL-terminated list of conversation ids. An empty whitelist matches
/// nothing; an entry matches when the clientid starts with it and the next character is `_`, the separator WeChat
/// puts between the conversation id and the rest of the clientid.
const TEMPLATE: [u8; 0xb4] = [
    0x50, 0x53, 0x51, 0x52, 0x56, 0x57, 0x41, 0x50, 0x41, 0x51, 0x41, 0x52, 0x41, 0x53, 0x48, 0x81,
    0xfa, 0x00, 0x00, 0x01, 0x00, 0x72, 0x5e, 0x48, 0x8b, 0x72, 0x50, 0x48, 0x85, 0xf6, 0x74, 0x55,
    0x48, 0x8d, 0x1d, 0xd9, 0x02, 0x00, 0x00, 0x80, 0x3b, 0x00, 0x74, 0x49, 0x49, 0x89, 0xf1, 0x49,
    0x89, 0xda, 0x41, 0x8a, 0x02, 0x84, 0xc0, 0x74, 0x0d, 0x41, 0x38, 0x01, 0x75, 0x0e, 0x49, 0xff,
    0xc1, 0x49, 0xff, 0xc2, 0xeb, 0xec, 0x41, 0x80, 0x39, 0x5f, 0x74, 0x14, 0x80, 0x3b, 0x00, 0x74,
    0x05, 0x48, 0xff, 0xc3, 0xeb, 0xf6, 0x48, 0xff, 0xc3, 0x80, 0x3b, 0x00, 0x74, 0x17, 0xeb, 0xcc,
    0x8b, 0x82, 0xd0, 0x00, 0x00, 0x00, 0x83, 0xf8, 0x03, 0x75, 0x0a, 0xc7, 0x82, 0xd0, 0x00, 0x00,
    0x00, 0x02, 0x00, 0x00, 0x00, 0x41, 0x5b, 0x41, 0x5a, 0x41, 0x59, 0x41, 0x58, 0x5f, 0x5e, 0x5a,
    0x59, 0x5b, 0x58, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90,
    0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90,
    0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90,
    0x90, 0x90, 0x90, 0x90,
];

/// Where the whitelist is written in the shellcode's page.
pub const WHITELIST_OFFSET: usize = 0x300;

/// Where a pending download request waits: a `1` at the flag offset asks the shellcode to run the request's
/// params once, on the next call, inside WeChat's own thread; the params are a whole download param object.
pub const REQUEST_FLAG_OFFSET: usize = 0x2f8;
pub const REQUEST_PARAM_OFFSET: usize = 0x400;
pub const REQUEST_PARAM_LEN: usize = 0x600;

/// Where the request block sits in the template: the padding after the register pops, so it runs with the entry
/// registers restored and falls through to the original first 13 bytes.
const REQUEST_BLOCK_OFFSET: usize = 0x83;

/// The bytes that replace the target function's first 13 bytes: `mov r10, <shellcode>; jmp r10`.
pub fn jump_to(shellcode: u64) -> [u8; 13] {
    let mut bytes = [0u8; 13];
    bytes[0] = 0x49;
    bytes[1] = 0xba;
    bytes[2..10].copy_from_slice(&shellcode.to_le_bytes());
    bytes[10] = 0x41;
    bytes[11] = 0xff;
    bytes[12] = 0xe2;
    bytes
}

/// The shellcode for a target whose first 13 bytes are `original`, placed at `page`: the template with the request
/// block filled in, those bytes, and a jump back to `target + 13`.
pub fn build(original: &[u8; 13], target: u64, page: u64) -> Vec<u8> {
    let mut code = Vec::with_capacity(0xcf);
    code.extend_from_slice(&TEMPLATE);
    let mut block = Vec::with_capacity(0x30);
    // mov r10, page + REQUEST_FLAG_OFFSET; cmp qword [r10], 0; je around the block
    block.extend_from_slice(&[0x49, 0xba]);
    block.extend_from_slice(&(page + REQUEST_FLAG_OFFSET as u64).to_le_bytes());
    block.extend_from_slice(&[0x49, 0x83, 0x3a, 0x00, 0x74, 0x21]);
    // the flag is taken, so this call runs the request once: clear it, then call the original function through the
    // bytes below with the request's params.
    block.extend_from_slice(&[0x49, 0xc7, 0x02, 0x00, 0x00, 0x00, 0x00, 0x52]);
    block.extend_from_slice(&[0x48, 0xba]);
    block.extend_from_slice(&(page + REQUEST_PARAM_OFFSET as u64).to_le_bytes());
    block.extend_from_slice(&[0x48, 0xb8]);
    block.extend_from_slice(&(page + 0xb4).to_le_bytes());
    block.extend_from_slice(&[0xff, 0xd0, 0x5a]);
    block.resize(0x30, 0x90);
    code[REQUEST_BLOCK_OFFSET..REQUEST_BLOCK_OFFSET + 0x30].copy_from_slice(&block);
    code.extend_from_slice(original);
    // jmp qword ptr [rip+0], with the address right after it
    code.extend_from_slice(&[0xff, 0x25, 0x00, 0x00, 0x00, 0x00]);
    code.extend_from_slice(&(target + 13).to_le_bytes());
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shellcode_has_the_layout_the_hook_expects() {
        let code = build(&[0xcc; 13], 0x1_0000_0000, 0x2_0000_0000);
        assert_eq!(code.len(), 0xcf);
        assert_eq!(&code[..2], &[0x50, 0x53]);
        // the whitelist is at +0x300, reached by the lea at +0x20
        assert_eq!(
            &code[0x20..0x27],
            &[0x48, 0x8d, 0x1d, 0xd9, 0x02, 0x00, 0x00]
        );
        assert_eq!(0x20 + 7 + 0x2d9, WHITELIST_OFFSET);
        // the original bytes, then the jump back to target + 13
        assert_eq!(&code[0xb4..0xc1], &[0xcc; 13]);
        assert_eq!(&code[0xc1..0xc7], &[0xff, 0x25, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(&code[0xc7..0xcf], &0x1_0000_000d_u64.to_le_bytes());
    }

    #[test]
    fn the_request_block_points_at_the_flag_params_and_original_bytes() {
        let page = 0x2_0000_0000;
        let code = build(&[0xcc; 13], 0x1_0000_0000, page);
        assert_eq!(&code[0x83..0x85], &[0x49, 0xba]);
        assert_eq!(
            u64::from_le_bytes(code[0x85..0x8d].try_into().unwrap()),
            page + REQUEST_FLAG_OFFSET as u64
        );
        assert_eq!(
            u64::from_le_bytes(code[0x9d..0xa5].try_into().unwrap()),
            page + REQUEST_PARAM_OFFSET as u64
        );
        assert_eq!(
            u64::from_le_bytes(code[0xa7..0xaf].try_into().unwrap()),
            page + 0xb4
        );
    }

    #[test]
    fn jump_is_mov_r10_then_jmp_r10() {
        let bytes = jump_to(0x1234_5678_9abc_def0);
        assert_eq!(&bytes[..2], &[0x49, 0xba]);
        assert_eq!(&bytes[2..10], &0x1234_5678_9abc_def0_u64.to_le_bytes());
        assert_eq!(&bytes[10..], &[0x41, 0xff, 0xe2]);
    }
}

#[cfg(all(test, windows, target_arch = "x86_64"))]
mod run_tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{
        VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE,
    };

    /// Runs the shellcode against a fake download object and returns the download type it left at `+0xd0`.
    fn run(whitelist: &[u8], conversation: Option<&[u8]>, download_type: u32) -> u32 {
        // SAFETY: a fresh page of executable memory, freed at the end.
        let page = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                0x1000,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        } as *mut u8;
        assert!(!page.is_null());
        // a `ret` for the shellcode to jump back to
        // SAFETY: `page` is a 0x1000-byte allocation, so `page + 0x800` is inside it.
        let ret = unsafe { page.add(0x800) };
        // SAFETY: `ret` is inside the allocation.
        unsafe { *ret = 0xc3 };
        let code = build(&[0x90; 13], ret as u64 - 13, page as u64);
        // SAFETY: the page is 0x1000 bytes and the code and whitelist fit in it.
        unsafe {
            std::ptr::copy_nonoverlapping(code.as_ptr(), page, code.len());
            std::ptr::copy_nonoverlapping(
                whitelist.as_ptr(),
                page.add(WHITELIST_OFFSET),
                whitelist.len(),
            );
        }
        let mut object = [0u8; 0x200];
        // the conversation id is a NUL-terminated C string
        let id = conversation.map(|id| {
            let mut id = id.to_vec();
            id.push(0);
            id
        });
        if let Some(id) = &id {
            object[0x50..0x58].copy_from_slice(&(id.as_ptr() as u64).to_le_bytes());
        }
        object[0xd0..0xd4].copy_from_slice(&download_type.to_le_bytes());
        // SAFETY: the page holds the shellcode; the second argument is the object it reads.
        let shellcode: extern "C" fn(*mut u8, *mut u8) = unsafe { std::mem::transmute(page) };
        shellcode(std::ptr::null_mut(), object.as_mut_ptr());
        let result = u32::from_le_bytes([object[0xd0], object[0xd1], object[0xd2], object[0xd3]]);
        // SAFETY: the page was allocated above and is not used again.
        unsafe { VirtualFree(page as *mut _, 0, MEM_RELEASE) };
        result
    }

    #[test]
    fn a_whitelisted_conversation_gets_the_original() {
        // WeChat builds the clientid as `<conversation id>_<timestamp>_<number>`
        assert_eq!(run(b"convA\0\0", Some(b"convA_1792635905_32675_3"), 3), 2);
        assert_eq!(run(b"convA\0convB\0\0", Some(b"convB_1_2_3"), 3), 2);
    }

    #[test]
    fn other_conversations_keep_their_download_type() {
        assert_eq!(run(b"convA\0\0", Some(b"convB_1_2_3"), 3), 3);
        assert_eq!(run(b"convA\0convB\0\0", Some(b"convC_1_2_3"), 3), 3);
        // the entry has to be followed by `_`, not merely be a prefix
        assert_eq!(run(b"convA\0\0", Some(b"convAB_1_2_3"), 3), 3);
        assert_eq!(run(b"convAB\0\0", Some(b"convA_1_2_3"), 3), 3);
        // a clientid that is only the conversation id does not match either
        assert_eq!(run(b"convA\0\0", Some(b"convA"), 3), 3);
    }

    #[test]
    fn an_empty_whitelist_matches_nothing() {
        assert_eq!(run(b"\0\0", Some(b"convA_1_2_3"), 3), 3);
    }

    #[test]
    fn a_pending_request_runs_the_original_function_inside_the_hook() {
        // SAFETY: a fresh page of executable memory, freed at the end.
        let page = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                0x2000,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            )
        } as *mut u8;
        assert!(!page.is_null());
        let target = unsafe { page.add(0x1000) };
        let marker = unsafe { page.add(0x1800) };
        // SAFETY: `marker` is inside the allocation.
        unsafe { marker.cast::<u64>().write(0) };
        // the fake function: 13 bytes the hook replays, then a body that counts its calls
        for i in 0..13 {
            // SAFETY: inside the allocation.
            unsafe { target.add(i).write(0x90) };
        }
        let body = unsafe { target.add(13) };
        let mut body_code = vec![0x48, 0xb8];
        body_code.extend_from_slice(&(marker as u64).to_le_bytes());
        body_code.extend_from_slice(&[0x48, 0xff, 0x00, 0xc3]);
        // SAFETY: `body` is inside the allocation and `body_code` fits.
        unsafe { std::ptr::copy_nonoverlapping(body_code.as_ptr(), body, body_code.len()) };
        let shellcode = build(&[0x90; 13], target as u64, page as u64);
        // SAFETY: the page is 0x2000 bytes and the shellcode fits.
        unsafe { std::ptr::copy_nonoverlapping(shellcode.as_ptr(), page, shellcode.len()) };
        // a pending request: the flag and the params
        // SAFETY: both offsets are inside the allocation.
        unsafe {
            page.add(REQUEST_FLAG_OFFSET).cast::<u64>().write(1);
            page.add(REQUEST_PARAM_OFFSET).cast::<u64>().write(0);
        }
        // patch the fake function's entry with the jump the hook writes
        let jump = jump_to(page as u64);
        // SAFETY: `target` is the fake function's first 13 bytes.
        unsafe { std::ptr::copy_nonoverlapping(jump.as_ptr(), target, 13) };
        let mut object = [0u8; 0x200];
        // SAFETY: the page holds the shellcode; the second argument is the object it reads.
        let call: extern "C" fn(*mut u8, *mut u8) = unsafe { std::mem::transmute(target) };
        call(std::ptr::null_mut(), object.as_mut_ptr());
        // the request call and the replay of the original 13 bytes both ran the body
        // SAFETY: `marker` is inside the allocation.
        assert_eq!(unsafe { marker.cast::<u64>().read() }, 2);
        // SAFETY: inside the allocation.
        assert_eq!(
            unsafe { page.add(REQUEST_FLAG_OFFSET).cast::<u64>().read() },
            0,
            "the flag is cleared before the request runs"
        );
        // SAFETY: the page was allocated above and is not used again.
        unsafe { VirtualFree(page as *mut _, 0, MEM_RELEASE) };
    }

    #[test]
    fn only_the_normal_download_type_is_changed() {
        assert_eq!(run(b"convA\0\0", Some(b"convA_1_2_3"), 2), 2);
        assert_eq!(run(b"convA\0\0", Some(b"convA_1_2_3"), 1), 1);
        assert_eq!(run(b"convA\0\0", None, 3), 3);
    }
}
