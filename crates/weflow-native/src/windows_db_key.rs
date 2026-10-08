//! Windows x64 database-key capture. Signatures and the RDX key structure come from
//! ycccccccy/wx_key (MIT); capture uses documented Windows debugging APIs instead of
//! injected code. The original function and the client's memory are never patched.

#[cfg(all(windows, target_arch = "x86_64"))]
mod platform;
#[cfg(all(windows, target_arch = "x86_64"))]
pub use platform::Hook;

// Scan only committed executable regions of Weixin.dll. A version must select exactly
// one signature, and there must be exactly one candidate before attaching a debugger.
#[derive(Clone, Copy)]
struct Signature {
    bytes: &'static [u8],
    wildcards: &'static [usize],
    back: usize,
}

fn signature(version: [u16; 4]) -> Option<Signature> {
    if version[0] != 4 {
        return None;
    }
    if version >= [4, 1, 4, 0] {
        Some(Signature {
            bytes: &[
                0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x00, 0x18, 0x48, 0x89,
                0x7c, 0x00, 0x20, 0x41, 0x56, 0x48, 0x83, 0xec, 0x50, 0x41,
            ],
            wildcards: &[10, 15],
            back: 3,
        })
    } else if version[1] <= 1 {
        Some(Signature {
            bytes: &[
                0x24, 0x50, 0x48, 0xc7, 0x45, 0x00, 0xfe, 0xff, 0xff, 0xff, 0x44, 0x89, 0xcf, 0x44,
                0x89, 0xc3, 0x49, 0x89, 0xd6, 0x48, 0x89, 0xce, 0x48, 0x89,
            ],
            wildcards: &[],
            back: 15,
        })
    } else {
        None
    }
}

fn matches(data: &[u8], sig: Signature) -> impl Iterator<Item = usize> + '_ {
    data.windows(sig.bytes.len())
        .enumerate()
        .filter_map(move |(i, bytes)| {
            bytes
                .iter()
                .zip(sig.bytes)
                .enumerate()
                .all(|(j, (a, b))| a == b || sig.wildcards.contains(&j))
                .then_some(i)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_boundary_and_supported_major() {
        assert_eq!(signature([4, 0, 9, 1]).unwrap().back, 15);
        assert_eq!(signature([4, 1, 3, 99]).unwrap().back, 15);
        assert_eq!(signature([4, 1, 4, 0]).unwrap().back, 3);
        assert_eq!(signature([4, 1, 5, 1]).unwrap().back, 3);
        assert!(signature([3, 9, 12, 0]).is_none());
        assert!(signature([5, 0, 0, 0]).is_none());
    }

    #[test]
    fn scan_handles_short_input_last_offset_and_ambiguity() {
        let sig = signature([4, 1, 4, 0]).unwrap();
        assert_eq!(matches(&[], sig).count(), 0);
        assert_eq!(matches(&[0; 5], sig).count(), 0);
        let mut bytes = vec![0; 7];
        bytes.extend_from_slice(sig.bytes);
        bytes[7 + 10] = 0xab;
        bytes[7 + 15] = 0xcd;
        assert_eq!(matches(&bytes, sig).collect::<Vec<_>>(), [7]);
        bytes.extend_from_slice(sig.bytes);
        assert_eq!(matches(&bytes, sig).count(), 2);
        bytes[7 + 9] ^= 1;
        assert_eq!(matches(&bytes, sig).collect::<Vec<_>>(), [31]);
    }
}
