//! The Windows x64 side of the image auto-download hook: find `Weixin.dll` in the WeChat process, locate the
//! function that downloads media, and patch it to run [`shellcode`] first.
//!
//! This is the Rust replacement for `img_helper.dll`. It uses only system DLLs, so it does not need the Visual C++
//! runtime.

#[cfg(all(windows, target_arch = "x86_64"))]
mod imp {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE, HMODULE};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        FlushInstructionCache, ReadProcessMemory, WriteProcessMemory,
    };
    use windows_sys::Win32::System::Memory::{
        VirtualAllocEx, VirtualFreeEx, VirtualProtectEx, VirtualQueryEx, MEMORY_BASIC_INFORMATION,
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, PAGE_GUARD, PAGE_NOACCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        K32EnumProcessModules, K32GetModuleBaseNameW, K32GetModuleInformation, MODULEINFO,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_ALL_ACCESS};

    use crate::image_download::shellcode;

    /// The bytes the hook replaces at the start of the target function.
    const PATCH_LEN: usize = 13;
    /// How far back from the `lea` the function's `push rbp` may be.
    const PROLOGUE_SEARCH: usize = 0x800;

    pub fn supported() -> bool {
        true
    }

    /// A hook that is installed in a running WeChat process.
    pub struct Hook {
        process: usize,
        target: usize,
        original: [u8; PATCH_LEN],
        shellcode: usize,
        /// The shellcode's page: the whitelist, and where a pending download request waits.
        page: usize,
    }

    impl Hook {
        /// Hooks the main `Weixin.exe` process `pid`, making it download originals for `whitelist` (NUL-separated,
        /// double-NUL-terminated; empty for none).
        pub fn install(pid: u32, whitelist: &[u8]) -> Result<Self, String> {
            // SAFETY: `OpenProcess` only opens a handle; the handle is closed on every path below.
            let process = unsafe { OpenProcess(PROCESS_ALL_ACCESS, FALSE, pid) };
            if process.is_null() {
                return Err("OpenProcess failed".into());
            }
            match Self::install_with(process, whitelist) {
                Ok(hook) => Ok(hook),
                Err(err) => {
                    // SAFETY: `process` is the handle opened above and is not used again.
                    unsafe { CloseHandle(process) };
                    Err(err)
                }
            }
        }

        fn install_with(process: HANDLE, whitelist: &[u8]) -> Result<Self, String> {
            let (base, size) = find_module(process).ok_or("Weixin.dll not found")?;
            let memory = read_module(process, base, size).ok_or("Memory read failed")?;
            let target = find_target(&memory, base).ok_or("Target function not found")?;

            let mut original = [0u8; PATCH_LEN];
            // SAFETY: `target` is inside the module we just read; the buffer is `PATCH_LEN` bytes.
            let read = unsafe {
                ReadProcessMemory(
                    process,
                    target as *const c_void,
                    original.as_mut_ptr() as *mut c_void,
                    PATCH_LEN,
                    std::ptr::null_mut(),
                )
            };
            if read == 0 {
                return Err("Backup failed".into());
            }

            // The shellcode reads the whitelist at +0x300; an empty whitelist is a single NUL.
            let whitelist: &[u8] = if whitelist.is_empty() {
                &[0u8][..]
            } else {
                whitelist
            };
            // The page holds the code, the whitelist, and a pending download request with its strings.
            let size = (shellcode::REQUEST_STRING_OFFSET + 4 * shellcode::REQUEST_STRING_ROOM)
                .max(shellcode::WHITELIST_OFFSET + whitelist.len() + 1);
            // SAFETY: allocating a page in the target process; freed in `uninstall`.
            let remote = unsafe {
                VirtualAllocEx(
                    process,
                    std::ptr::null(),
                    size,
                    MEM_COMMIT | MEM_RESERVE,
                    PAGE_EXECUTE_READWRITE,
                )
            };
            let code = shellcode::build(&original, target, remote as u64);
            if remote.is_null() {
                return Err("VirtualAllocEx failed".into());
            }

            let mut written = 0usize;
            // SAFETY: `remote` is a page of at least `code.len()` bytes in the target process.
            let ok = unsafe {
                WriteProcessMemory(
                    process,
                    remote,
                    code.as_ptr() as *const c_void,
                    code.len(),
                    &mut written,
                )
            };
            if ok == 0 {
                // SAFETY: `remote` was allocated above and is not used again.
                unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };
                return Err("Shellcode write failed".into());
            }

            // SAFETY: the allocation is large enough for the whitelist at +0x300.
            let ok = unsafe {
                WriteProcessMemory(
                    process,
                    (remote as usize + shellcode::WHITELIST_OFFSET) as *const c_void,
                    whitelist.as_ptr() as *const c_void,
                    whitelist.len(),
                    &mut written,
                )
            };
            if ok == 0 {
                // SAFETY: `remote` was allocated above and is not used again.
                unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };
                return Err("Whitelist write failed".into());
            }

            let mut old = 0u32;
            // SAFETY: `target` is the function's first `PATCH_LEN` bytes.
            let ok = unsafe {
                VirtualProtectEx(
                    process,
                    target as *const c_void,
                    PATCH_LEN,
                    PAGE_EXECUTE_READWRITE,
                    &mut old,
                )
            };
            if ok == 0 {
                // SAFETY: `remote` was allocated above and is not used again.
                unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };
                return Err("VirtualProtectEx failed".into());
            }

            let jump = shellcode::jump_to(remote as u64);
            // SAFETY: `target` is writable for `PATCH_LEN` bytes after the call above.
            let ok = unsafe {
                WriteProcessMemory(
                    process,
                    target as *const c_void,
                    jump.as_ptr() as *const c_void,
                    PATCH_LEN,
                    &mut written,
                )
            };
            // SAFETY: restoring the protection the function had; `old` was filled in above.
            unsafe { VirtualProtectEx(process, target as *const c_void, PATCH_LEN, old, &mut old) };
            if ok == 0 {
                // SAFETY: `remote` was allocated above and is not used again.
                unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };
                return Err("Hook write failed".into());
            }
            // SAFETY: the patched code is in the target process.
            unsafe { FlushInstructionCache(process, target as *const c_void, PATCH_LEN) };

            Ok(Self {
                process: process as usize,
                target: usize::try_from(target).unwrap_or(0),
                original,
                shellcode: remote as usize,
                page: remote as usize,
            })
        }

        /// Leaves a download request in the shellcode's page: the next `_startDownloadMedia` call runs it once,
        /// inside WeChat's own thread and with WeChat's own `CdnCore` instance.
        pub fn request_original(
            &self,
            clientid: &str,
            file_id: &str,
            aeskey: &str,
            fullpath: &str,
            file_len: u64,
        ) -> Result<(), String> {
            let process = self.process as HANDLE;
            let mut written = 0usize;
            let mut addresses = Vec::with_capacity(4);
            for (i, text) in [clientid, file_id, aeskey, fullpath]
                .into_iter()
                .enumerate()
            {
                let bytes = text.as_bytes();
                if bytes.len() + 1 > shellcode::REQUEST_STRING_ROOM {
                    return Err("request string too long".into());
                }
                let address = self.page as u64
                    + (shellcode::REQUEST_STRING_OFFSET + i * shellcode::REQUEST_STRING_ROOM)
                        as u64;
                let mut data = bytes.to_vec();
                data.push(0);
                // SAFETY: the page is large enough for each string's room, and the address is the one reserved
                // for it.
                let ok = unsafe {
                    WriteProcessMemory(
                        process,
                        address as *const c_void,
                        data.as_ptr() as *const c_void,
                        data.len(),
                        &mut written,
                    )
                };
                if ok == 0 {
                    return Err("Request string write failed".into());
                }
                addresses.push((address, bytes.len() as u64));
            }
            let param = shellcode::request_param(
                addresses[0],
                addresses[1],
                addresses[2],
                addresses[3],
                file_len,
            );
            // SAFETY: the page is large enough for the params, and the flag comes after them; both writes go to the
            // process this hook is installed in.
            unsafe {
                let ok = WriteProcessMemory(
                    process,
                    (self.page + shellcode::REQUEST_PARAM_OFFSET) as *const c_void,
                    param.as_ptr() as *const c_void,
                    param.len(),
                    &mut written,
                );
                if ok == 0 {
                    return Err("Request write failed".into());
                }
                let flag = 1u64;
                let ok = WriteProcessMemory(
                    process,
                    (self.page + shellcode::REQUEST_FLAG_OFFSET) as *const c_void,
                    (&flag as *const u64) as *const c_void,
                    8,
                    &mut written,
                );
                if ok == 0 {
                    return Err("Request flag write failed".into());
                }
            }
            Ok(())
        }

        /// Restores the original bytes and frees the shellcode page.
        pub fn uninstall(&self) {
            let process = self.process as HANDLE;
            let target = self.target as *const c_void;
            let mut written = 0usize;
            // SAFETY: the handle and addresses are the ones `install` kept; the original bytes are `PATCH_LEN` long.
            unsafe {
                WriteProcessMemory(
                    process,
                    target,
                    self.original.as_ptr() as *const c_void,
                    PATCH_LEN,
                    &mut written,
                );
                FlushInstructionCache(process, target, PATCH_LEN);
                VirtualFreeEx(process, self.shellcode as *mut c_void, 0, MEM_RELEASE);
                CloseHandle(process);
            }
        }
    }

    /// The `Weixin.dll` module of `process`: its base address and image size.
    fn find_module(process: HANDLE) -> Option<(u64, usize)> {
        let mut modules = [std::ptr::null_mut::<c_void>(); 1024];
        let mut needed = 0u32;
        // SAFETY: `modules` is a buffer of `size_of_val` bytes; `needed` receives the count.
        let ok = unsafe {
            K32EnumProcessModules(
                process,
                modules.as_mut_ptr(),
                u32::try_from(std::mem::size_of_val(&modules)).unwrap_or(u32::MAX),
                &mut needed,
            )
        };
        if ok == 0 {
            return None;
        }
        let count = (needed as usize / std::mem::size_of::<HMODULE>()).min(modules.len());
        for &module in &modules[..count] {
            let mut name = [0u16; 260];
            // SAFETY: `name` is a buffer of 260 wide characters.
            let len = unsafe {
                K32GetModuleBaseNameW(
                    process,
                    module,
                    name.as_mut_ptr(),
                    u32::try_from(name.len()).unwrap_or(u32::MAX),
                )
            };
            if len == 0 {
                continue;
            }
            let name = &name[..len as usize];
            if !eq_ignore_case(name, &wide("Weixin.dll")) {
                continue;
            }
            let mut info = MODULEINFO::default();
            // SAFETY: `info` is a `MODULEINFO` and `module` is one of the process's modules.
            let ok = unsafe {
                K32GetModuleInformation(
                    process,
                    module,
                    &mut info,
                    u32::try_from(std::mem::size_of::<MODULEINFO>()).unwrap_or(u32::MAX),
                )
            };
            if ok == 0 {
                return None;
            }
            return Some((info.lpBaseOfDll as u64, info.SizeOfImage as usize));
        }
        None
    }

    /// Reads the module's committed, readable memory into a buffer indexed by offset from `base`; regions that
    /// cannot be read stay zero.
    fn read_module(process: HANDLE, base: u64, size: usize) -> Option<Vec<u8>> {
        let mut buffer = vec![0u8; size];
        let end = base.checked_add(size as u64)?;
        let mut address = base;
        while address < end {
            let mut info = MEMORY_BASIC_INFORMATION::default();
            // SAFETY: `info` is a `MEMORY_BASIC_INFORMATION` of the right size.
            let written = unsafe {
                VirtualQueryEx(
                    process,
                    address as *const c_void,
                    &mut info,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if written == 0 {
                break;
            }
            let region_start = info.BaseAddress as u64;
            let region_end = region_start.saturating_add(info.RegionSize as u64);
            let read_start = region_start.max(address);
            let read_end = region_end.min(end);
            if read_start < read_end
                && info.State == MEM_COMMIT
                && (info.Protect & PAGE_NOACCESS) == 0
                && (info.Protect & PAGE_GUARD) == 0
            {
                let offset = usize::try_from(read_start - base).unwrap_or(0);
                let len = usize::try_from(read_end - read_start).unwrap_or(0);
                // SAFETY: the destination is inside `buffer` and `len` bytes long.
                unsafe {
                    ReadProcessMemory(
                        process,
                        read_start as *const c_void,
                        buffer[offset..].as_mut_ptr() as *mut c_void,
                        len,
                        std::ptr::null_mut(),
                    );
                }
            }
            if region_end <= address {
                break;
            }
            address = region_end;
        }
        Some(buffer)
    }

    /// The function that downloads media: the one whose `lea reg, [rip+disp]` points at the
    /// `_startDownloadMedia` string (or at the `mars::cdn::CdnCore::` prefix in front of it), starting at the
    /// nearest `push rbp` before it.
    fn find_target(module: &[u8], base: u64) -> Option<u64> {
        let mut string = find_bytes(module, b"_startDownloadMedia")?;
        // The string is usually the tail of `mars::cdn::CdnCore::_startDownloadMedia`, and the `lea` points at the
        // whole string; step back over the prefix when it is there.
        const PREFIX: &[u8] = b"mars::cdn::CdnCore::";
        if string >= PREFIX.len() && &module[string - PREFIX.len()..string] == PREFIX {
            string -= PREFIX.len();
        }
        let string_addr = base + string as u64;
        for i in 0..module.len().saturating_sub(7) {
            let b0 = module[i];
            let b1 = module[i + 1];
            let b2 = module[i + 2];
            // `lea r64, [rip+disp32]`: REX.W, 0x8d, and a ModRM with mod=00, rm=101 (not rsp's SIB form).
            let is_lea = (b0 == 0x48 || b0 == 0x4c)
                && b1 == 0x8d
                && (b2 == 0x05 || ((b2.wrapping_sub(0x0d) & 0xe7) == 0 && b2 != 0x25));
            if !is_lea {
                continue;
            }
            let disp =
                i32::from_le_bytes([module[i + 3], module[i + 4], module[i + 5], module[i + 6]]);
            let target = (base + i as u64)
                .wrapping_add(7)
                .wrapping_add(disp as i64 as u64);
            if target != string_addr {
                continue;
            }
            let start = i.saturating_sub(PROLOGUE_SEARCH);
            for j in (start..=i).rev() {
                if module[j] == 0x55 {
                    return Some(base + j as u64);
                }
            }
        }
        None
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        if needle.is_empty() || haystack.len() < needle.len() {
            return None;
        }
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn eq_ignore_case(a: &[u16], b: &[u16]) -> bool {
        a.len() == b.len()
            && a.iter().zip(b).all(|(x, y)| {
                u8::try_from(*x)
                    .unwrap_or(0)
                    .eq_ignore_ascii_case(&u8::try_from(*y).unwrap_or(0))
            })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn finds_the_function_through_the_lea_and_push_rbp() {
            // `push rbp` at 0x10, a `lea rbx, [rip+disp]` at 0x20 pointing at the string at 0x40.
            let mut module = vec![0u8; 0x80];
            module[0x10] = 0x55;
            module[0x20..0x27].copy_from_slice(&[0x48, 0x8d, 0x1d, 0x19, 0x00, 0x00, 0x00]);
            module[0x40..0x40 + 19].copy_from_slice(b"_startDownloadMedia");
            assert_eq!(find_target(&module, 0x1000), Some(0x1010));
        }

        #[test]
        fn finds_the_function_when_the_lea_points_at_the_whole_string() {
            // the same, but the string is `mars::cdn::CdnCore::_startDownloadMedia` and the `lea` points at its start
            let mut module = vec![0u8; 0x80];
            module[0x10] = 0x55;
            module[0x20..0x27].copy_from_slice(&[0x48, 0x8d, 0x1d, 0x19, 0x00, 0x00, 0x00]);
            module[0x40..0x40 + 20].copy_from_slice(b"mars::cdn::CdnCore::");
            module[0x54..0x54 + 19].copy_from_slice(b"_startDownloadMedia");
            assert_eq!(find_target(&module, 0x1000), Some(0x1010));
        }

        #[test]
        fn no_target_without_the_string() {
            let module = vec![0u8; 0x80];
            assert_eq!(find_target(&module, 0x1000), None);
        }
    }
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
mod imp {
    pub fn supported() -> bool {
        false
    }

    /// The hook is only implemented for Windows x64; elsewhere it is never installed.
    pub struct Hook;

    impl Hook {
        pub fn install(_pid: u32, _whitelist: &[u8]) -> Result<Self, String> {
            Err("image auto download is only supported on Windows x64".into())
        }

        pub fn request_original(
            &self,
            _clientid: &str,
            _file_id: &str,
            _aeskey: &str,
            _fullpath: &str,
            _file_len: u64,
        ) -> Result<(), String> {
            Err("image auto download is only supported on Windows x64".into())
        }

        pub fn uninstall(&self) {}
    }
}

pub use imp::{supported, Hook};
