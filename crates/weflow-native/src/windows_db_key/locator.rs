//! 在微信的模块中找到捕获密钥的位置：模块的 PE 节、函数表，以及每个微信版本的字节特征码。

use anyhow::{bail, Context, Result};
use std::ops::Range;

/// 模块中的一个节：起点（相对映像基址）和长度。
#[derive(Debug, Clone, Copy)]
pub(super) struct Section {
    pub rva: usize,
    pub size: usize,
}

/// 模块中 `.text` 节和 AMD64 异常目录的位置。
pub(super) struct PeLayout {
    pub text: Section,
    pub exceptions: Section,
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .context("truncated PE header")?
            .try_into()?,
    ))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .context("truncated PE header")?
            .try_into()?,
    ))
}
fn section(rva: usize, size: usize, image_size: usize) -> Result<Section> {
    if size == 0 || rva.checked_add(size).is_none_or(|end| end > image_size) {
        bail!("PE section is outside Weixin.dll");
    }
    Ok(Section { rva, size })
}

/// 从模块开头的字节 `headers` 读出 [`PeLayout`]，`image_size` 是模块映像的大小。
///
/// 映像必须是 AMD64 的 PE32+，`SizeOfImage` 等于 `image_size`；异常目录（数据目录的第 4 项）不为空，
/// 大小是 12 字节（一条 `RUNTIME_FUNCTION`）的整数倍；恰好有一个 `.text` 节且可读可执行；各个位置都在映像之内。
/// 不满足时返回错误。
pub(super) fn pe_layout(headers: &[u8], image_size: usize) -> Result<PeLayout> {
    if headers.get(..2) != Some(b"MZ") {
        bail!("invalid DOS header");
    }
    let pe = u32_at(headers, 0x3c)? as usize;
    if pe > headers.len().saturating_sub(24) || headers.get(pe..pe + 4) != Some(b"PE\0\0") {
        bail!("invalid PE signature");
    }
    if u16_at(headers, pe + 4)? != 0x8664 {
        bail!("Weixin.dll must be an AMD64 image");
    }
    let count = u16_at(headers, pe + 6)? as usize;
    let optional_size = u16_at(headers, pe + 20)? as usize;
    let optional = pe + 24;
    if count == 0
        || count > 96
        || optional_size < 144
        || u16_at(headers, optional)? != 0x20b
        || u32_at(headers, optional + 108)? < 4
    {
        bail!("invalid PE32+ optional header");
    }
    if u32_at(headers, optional + 56)? as usize != image_size {
        bail!("Weixin.dll image size changed during inspection");
    }
    let exceptions = section(
        u32_at(headers, optional + 136)? as usize,
        u32_at(headers, optional + 140)? as usize,
        image_size,
    )?;
    if !exceptions.size.is_multiple_of(12) {
        bail!("invalid AMD64 exception directory size");
    }
    let table = optional + optional_size;
    let end = table
        .checked_add(count * 40)
        .context("PE section table overflow")?;
    let mut text = None;
    for record in headers
        .get(table..end)
        .context("truncated PE section table")?
        .as_chunks::<40>()
        .0
    {
        if &record[..8] != b".text\0\0\0" {
            continue;
        }
        if text.is_some() || u32_at(record, 36)? & 0x60000000 != 0x60000000 {
            bail!("invalid executable PE section");
        }
        text = Some(section(
            u32_at(record, 12)? as usize,
            u32_at(record, 8)? as usize,
            image_size,
        )?);
    }
    Ok(PeLayout {
        text: text.context("Weixin.dll has no .text section")?,
        exceptions,
    })
}

/// 把异常目录的记录（每条 12 字节的 `RUNTIME_FUNCTION`）读成 `text` 节内的函数范围，按起点排序。
///
/// 跳过起止地址都为 0 的记录和 `text` 节以外的函数；记录被截断、范围为空或倒置，或者范围相互重叠时返回错误。
pub(super) fn function_ranges(bytes: &[u8], text: Section) -> Result<Vec<Range<usize>>> {
    if !bytes.len().is_multiple_of(12) {
        bail!("truncated AMD64 runtime function record");
    }
    let mut ranges = Vec::new();
    for record in bytes.as_chunks::<12>().0 {
        let range = u32_at(record, 0)? as usize..u32_at(record, 4)? as usize;
        if range.start == 0 && range.end == 0 {
            continue;
        }
        if range.start >= range.end {
            bail!("invalid AMD64 runtime function range");
        }
        if range.start >= text.rva && range.end <= text.rva + text.size {
            ranges.push(range);
        }
    }
    ranges.sort_unstable_by_key(|range| range.start);
    if ranges.windows(2).any(|pair| pair[0].end > pair[1].start) {
        bail!("overlapping AMD64 runtime function ranges");
    }
    Ok(ranges)
}

/// 在一段微信版本中标出捕获位置的字节。
#[derive(Clone, Copy)]
pub(super) struct Signature {
    pub bytes: &'static [u8],
    /// `bytes` 中可以匹配任意字节的位置。
    wildcards: &'static [usize],
    /// 捕获位置在匹配处之前多少字节。
    back: usize,
}

/// 4.1.4.0 之前和 4.1.6.14 之后的微信的特征码。
const OLD: &[u8] = &[
    0x24, 0x50, 0x48, 0xc7, 0x45, 0x00, 0xfe, 0xff, 0xff, 0xff, 0x44, 0x89, 0xcf, 0x44, 0x89, 0xc3,
    0x49, 0x89, 0xd6, 0x48, 0x89, 0xce, 0x48, 0x89,
];
/// 微信 4.1.4.0 到 4.1.6.14 的特征码。
const MIDDLE: &[u8] = &[
    0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x00, 0x18, 0x48, 0x89, 0x7c, 0x00,
    0x20, 0x41, 0x56, 0x48, 0x83, 0xec, 0x50, 0x41,
];

/// 微信 `version` 的特征码，按原版 `wx_key` 辅助程序的三段版本区间选择。
///
/// 4.1.6.14 之后的版本重新匹配 [`OLD`] 的字节，但捕获位置与匹配处的距离不同。
pub(super) fn signature(version: [u16; 4]) -> Result<Signature> {
    if version[0] != 4 {
        bail!("unsupported WeChat version: WeChat 4.x x64 is required");
    }
    Ok(if version > [4, 1, 6, 14] {
        Signature {
            bytes: OLD,
            wildcards: &[],
            back: 3,
        }
    } else if version >= [4, 1, 4, 0] {
        Signature {
            bytes: MIDDLE,
            wildcards: &[10, 15],
            back: 3,
        }
    } else {
        Signature {
            bytes: OLD,
            wildcards: &[],
            back: 15,
        }
    })
}

/// `bytes` 中与 `sig` 匹配的每个偏移，通配的位置可以是任意字节。
pub(super) fn matches(bytes: &[u8], sig: Signature) -> impl Iterator<Item = usize> + '_ {
    bytes
        .windows(sig.bytes.len())
        .enumerate()
        .filter_map(move |(offset, bytes)| {
            bytes
                .iter()
                .zip(sig.bytes)
                .enumerate()
                .all(|(index, (a, b))| a == b || sig.wildcards.contains(&index))
                .then_some(offset)
        })
}

/// 由特征码的匹配处 `matches` 算出捕获位置，即匹配处之前 `sig.back` 个字节。
///
/// 要求恰好有一处匹配，且捕获位置和匹配的最后一个字节都在 `functions` 中同一个函数的范围内，否则返回错误。
pub(super) fn capture_point(
    matches: &[usize],
    sig: Signature,
    functions: &[Range<usize>],
) -> Result<usize> {
    if matches.len() != 1 {
        bail!(
            "WeChat key signature matched {} candidates; expected exactly one",
            matches.len()
        );
    }
    let point = matches[0]
        .checked_sub(sig.back)
        .context("WeChat key signature offset underflow")?;
    let index = functions.partition_point(|function| function.start <= point);
    if !index
        .checked_sub(1)
        .and_then(|i| functions.get(i))
        .is_some_and(|range| {
            range.contains(&point) && range.contains(&(matches[0] + sig.bytes.len() - 1))
        })
    {
        bail!("WeChat key capture point is outside its AMD64 function range");
    }
    Ok(point)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_exact_binary_version_boundaries() {
        assert_eq!(signature([4, 0, 9, 1]).unwrap().back, 15);
        assert_eq!(signature([4, 1, 3, 99]).unwrap().back, 15);
        assert_eq!(signature([4, 1, 4, 0]).unwrap().bytes, MIDDLE);
        assert_eq!(signature([4, 1, 6, 14]).unwrap().bytes, MIDDLE);
        assert_eq!(signature([4, 1, 6, 15]).unwrap().bytes, OLD);
        assert_eq!(signature([4, 1, 13, 65]).unwrap().back, 3);
        assert!(signature([3, 9, 0, 0]).is_err());
        assert!(signature([5, 0, 0, 0]).is_err());
    }

    #[test]
    fn checks_wildcards_last_offset_ambiguity_and_function_bounds() {
        let sig = signature([4, 1, 6, 14]).unwrap();
        assert_eq!(matches(&[], sig).count(), 0);
        let mut bytes = vec![0; 16];
        bytes.extend_from_slice(sig.bytes);
        bytes[26] = 0xab;
        bytes[31] = 0xcd;
        assert_eq!(matches(&bytes, sig).collect::<Vec<_>>(), [16]);
        assert_eq!(
            capture_point(&[16], sig, std::slice::from_ref(&(0..40))).unwrap(),
            13
        );
        assert!(capture_point(&[16, 32], sig, std::slice::from_ref(&(0..80))).is_err());
        assert!(capture_point(&[2], sig, std::slice::from_ref(&(0..80))).is_err());
        assert!(capture_point(&[16], sig, std::slice::from_ref(&(14..40))).is_err());
        assert!(capture_point(&[16], sig, std::slice::from_ref(&(0..39))).is_err());
        bytes[25] ^= 1;
        assert_eq!(matches(&bytes, sig).count(), 0);
    }

    #[test]
    fn rejects_invalid_pe_headers_and_runtime_function_records() {
        assert!(pe_layout(&[], 0x4000).is_err());
        let mut bytes = vec![0; 0x200];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0xfffffff0u32.to_le_bytes());
        assert!(pe_layout(&bytes, 0x4000).is_err());
        bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        bytes[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        bytes[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
        bytes[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
        bytes[0xd0..0xd4].copy_from_slice(&0x4000u32.to_le_bytes());
        bytes[0x104..0x108].copy_from_slice(&16u32.to_le_bytes());
        bytes[0x120..0x124].copy_from_slice(&0x3000u32.to_le_bytes());
        bytes[0x124..0x128].copy_from_slice(&12u32.to_le_bytes());
        bytes[0x188..0x190].copy_from_slice(b".text\0\0\0");
        bytes[0x190..0x194].copy_from_slice(&0x100u32.to_le_bytes());
        bytes[0x194..0x198].copy_from_slice(&0x1000u32.to_le_bytes());
        bytes[0x1ac..0x1b0].copy_from_slice(&0x60000020u32.to_le_bytes());
        let layout = pe_layout(&bytes, 0x4000).unwrap();
        assert_eq!(layout.text.rva, 0x1000);
        assert_eq!(layout.exceptions.rva, 0x3000);
        assert_eq!(layout.exceptions.size, 12);
        assert!(pe_layout(&bytes[..0x1af], 0x4000).is_err());
        bytes[0x194..0x198].copy_from_slice(&0x4000u32.to_le_bytes());
        assert!(pe_layout(&bytes, 0x4000).is_err());
        let text = Section {
            rva: 0x1000,
            size: 0x100,
        };
        let mut functions = Vec::new();
        for value in [0x1000u32, 0x1080, 0x3050, 0x1080, 0x1100, 0x3060] {
            functions.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(
            function_ranges(&functions, text).unwrap(),
            [0x1000..0x1080, 0x1080..0x1100]
        );
        functions[12..16].copy_from_slice(&0x107fu32.to_le_bytes());
        assert!(function_ranges(&functions, text).is_err());
        assert!(function_ranges(&functions[..11], text).is_err());
    }
}
