//! The machine code `img_helper.dll` writes into `Weixin.dll` to make WeChat download original-size images.
//!
//! The hook replaces the first 13 bytes of the target function with `mov r10, <shellcode>; jmp r10`. The shellcode
//! saves the registers, checks whether the conversation id (the first argument's `+0x50` field) is one of the
//! whitelisted ids, and when it is, changes the download type at `+0xd0` from 3 to 2. It then runs the 13 bytes it
//! replaced and jumps back to the instruction after them.
//!
//! The relative jumps are fixed: the shellcode is always laid out the same way, with the whitelist at `+0x300` and
//! the original bytes at `+0xb4`. Only the original bytes and the address they return to are filled in at run time.

/// The shellcode up to the saved registers, with every relative jump already resolved.
///
/// The whitelist is a NUL-separated, double-NUL-terminated list of conversation ids. An empty whitelist matches
/// nothing; an entry matches only when the whole conversation id equals it.
const TEMPLATE: [u8; 0xb4] = [
    0x50, 0x53, 0x51, 0x52, 0x56, 0x57, 0x41, 0x50, 0x41, 0x51, 0x41, 0x52, 0x41, 0x53, 0x48, 0x81,
    0xfa, 0x00, 0x00, 0x01, 0x00, 0x72, 0x5e, 0x48, 0x8b, 0x72, 0x50, 0x48, 0x85, 0xf6, 0x74, 0x55,
    0x48, 0x8d, 0x1d, 0xd9, 0x02, 0x00, 0x00, 0x80, 0x3b, 0x00, 0x74, 0x49, 0x49, 0x89, 0xf1, 0x49,
    0x89, 0xda, 0x41, 0x8a, 0x02, 0x84, 0xc0, 0x74, 0x0d, 0x41, 0x38, 0x01, 0x75, 0x0e, 0x49, 0xff,
    0xc1, 0x49, 0xff, 0xc2, 0xeb, 0xec, 0x41, 0x80, 0x39, 0x00, 0x74, 0x14, 0x80, 0x3b, 0x00, 0x74,
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

/// The shellcode for a target whose first 13 bytes are `original`: the template, those bytes, and a jump back to
/// `target + 13`.
pub fn build(original: &[u8; 13], target: u64) -> Vec<u8> {
    let mut code = Vec::with_capacity(0xcf);
    code.extend_from_slice(&TEMPLATE);
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
        let code = build(&[0xcc; 13], 0x1_0000_0000);
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
        let code = build(&[0x90; 13], ret as u64 - 13);
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
        assert_eq!(run(b"convA\0\0", Some(b"convA"), 3), 2);
        assert_eq!(run(b"convA\0convB\0\0", Some(b"convB"), 3), 2);
    }

    #[test]
    fn other_conversations_keep_their_download_type() {
        assert_eq!(run(b"convA\0\0", Some(b"convB"), 3), 3);
        assert_eq!(run(b"convA\0convB\0\0", Some(b"convC"), 3), 3);
        // a prefix is not a match
        assert_eq!(run(b"convA\0\0", Some(b"convAB"), 3), 3);
        assert_eq!(run(b"convAB\0\0", Some(b"convA"), 3), 3);
    }

    #[test]
    fn an_empty_whitelist_matches_nothing() {
        assert_eq!(run(b"\0\0", Some(b"convA"), 3), 3);
    }

    #[test]
    fn only_the_normal_download_type_is_changed() {
        assert_eq!(run(b"convA\0\0", Some(b"convA"), 2), 2);
        assert_eq!(run(b"convA\0\0", Some(b"convA"), 1), 1);
        assert_eq!(run(b"convA\0\0", None, 3), 3);
    }
}
