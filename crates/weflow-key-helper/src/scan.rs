//! Bounds-checked ELF64/x86-64 locator from the documented two-hop RIP reference chain.
use std::collections::BTreeSet;

use anyhow::{anyhow, bail, Context, Result};

fn u16_at(data: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        data.get(at..at + 2)
            .context("truncated ELF field")?
            .try_into()?,
    ))
}
fn u64_at(data: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        data.get(at..at + 8)
            .context("truncated ELF field")?
            .try_into()?,
    ))
}

#[derive(Clone, Copy)]
pub struct Section<'a> {
    pub address: u64,
    pub data: &'a [u8],
}

fn section<'a>(data: &'a [u8], at: usize) -> Result<Section<'a>> {
    let address = u64_at(data, at + 16)?;
    let start = usize::try_from(u64_at(data, at + 24)?)?;
    let size = usize::try_from(u64_at(data, at + 32)?)?;
    let end = start.checked_add(size).context("ELF section overflow")?;
    Ok(Section {
        address,
        data: data.get(start..end).context("ELF section outside file")?,
    })
}

pub fn locate_elf(data: &[u8]) -> Result<u64> {
    if data.get(..6) != Some(b"\x7fELF\x02\x01") || u16_at(data, 18)? != 62 {
        bail!("only little-endian ELF64/x86-64 is supported");
    }
    let start = usize::try_from(u64_at(data, 40)?)?;
    let size = usize::from(u16_at(data, 58)?);
    let count = usize::from(u16_at(data, 60)?);
    let names_index = usize::from(u16_at(data, 62)?);
    if size < 64 || count == 0 || names_index >= count {
        bail!("invalid or unsupported ELF section table");
    }
    let end = start
        .checked_add(size.checked_mul(count).context("section table overflow")?)
        .context("section table overflow")?;
    data.get(start..end).context("truncated section table")?;
    let names = section(data, start + names_index * size)?.data;
    let mut text = None;
    let mut rodata = None;
    for index in 0..count {
        let at = start + index * size;
        let name_index = u32::from_le_bytes(data[at..at + 4].try_into()?) as usize;
        let tail = names.get(name_index..).context("invalid section name")?;
        let end = tail
            .iter()
            .position(|b| *b == 0)
            .context("unterminated section name")?;
        match &tail[..end] {
            b".text" => text = Some(section(data, at)?),
            b".rodata" => rodata = Some(section(data, at)?),
            _ => {}
        }
    }
    locate_sections(
        text.context("ELF .text not found")?,
        rodata.context("ELF .rodata not found")?,
    )
}

fn rip_target(text: Section<'_>, at: usize, opcode: &[u8; 3]) -> Option<u64> {
    let instruction = text.data.get(at..at.checked_add(7)?)?;
    if instruction[..3] != *opcode {
        return None;
    }
    let next = text.address.checked_add(at as u64)?.checked_add(7)?;
    next.checked_add_signed(i64::from(i32::from_le_bytes(
        instruction[3..7].try_into().ok()?,
    )))
}

pub fn locate_sections(text: Section<'_>, rodata: Section<'_>) -> Result<u64> {
    let anchor = b"com.Tencent.WCDB.Config.Cipher\0";
    let strings: BTreeSet<_> = rodata
        .data
        .windows(anchor.len())
        .enumerate()
        .filter(|(_, bytes)| *bytes == anchor)
        .filter_map(|(at, _)| rodata.address.checked_add(at as u64))
        .collect();
    if strings.is_empty() {
        bail!("WCDB cipher string not found");
    }
    let mut objects = BTreeSet::new();
    for at in 7..text.data.len() {
        if rip_target(text, at, &[0x48, 0x8d, 0x35]).is_some_and(|target| strings.contains(&target))
        {
            if let Some(target) = rip_target(text, at - 7, &[0x48, 0x8d, 0x3d]) {
                objects.insert(target);
            }
        }
    }
    let mut heads = BTreeSet::new();
    for at in 0..text.data.len() {
        if !rip_target(text, at, &[0x48, 0x8d, 0x35])
            .is_some_and(|target| objects.contains(&target))
        {
            continue;
        }
        for head in (at.saturating_sub(0x500)..=at).rev() {
            if text.data.get(head..head + 3) == Some(&[0x55, 0x41, 0x57]) {
                heads.insert(
                    text.address
                        .checked_add(head as u64)
                        .context("target address overflow")?,
                );
                break;
            }
        }
    }
    match heads.into_iter().collect::<Vec<_>>().as_slice() {
        [head] => Ok(*head),
        [] => Err(anyhow!("no matching database-key function")),
        _ => Err(anyhow!(
            "ambiguous database-key function; refusing to attach"
        )),
    }
}

/// Calculate load bias from a file mapping and PT_LOAD, handling both PIE and ET_EXEC.
pub fn runtime_address(elf: &[u8], maps: &str, target: u64, executable: &str) -> Result<u64> {
    let start = usize::try_from(u64_at(elf, 32)?)?;
    let size = usize::from(u16_at(elf, 54)?);
    let count = usize::from(u16_at(elf, 56)?);
    if size < 56 {
        bail!("invalid program header size");
    }
    let mut addresses = BTreeSet::new();
    let mut executable_ranges = Vec::new();
    for line in maps.lines() {
        // /proc/maps padding varies; parse the first five whitespace fields separately.
        let mut tokens = line.split_whitespace();
        let (Some(range), Some(permissions), Some(offset), Some(_device), Some(_inode)) = (
            tokens.next(),
            tokens.next(),
            tokens.next(),
            tokens.next(),
            tokens.next(),
        ) else {
            continue;
        };
        let path = tokens.collect::<Vec<_>>().join(" ");
        if path.strip_suffix(" (deleted)").unwrap_or(&path) != executable {
            continue;
        }
        let Some((mapped, end)) = range.split_once('-') else {
            continue;
        };
        let mapped = u64::from_str_radix(mapped, 16)?;
        let end = u64::from_str_radix(end, 16)?;
        if permissions.contains('x') {
            executable_ranges.push(mapped..end);
        }
        let offset = u64::from_str_radix(offset, 16)?;
        for index in 0..count {
            let at = start
                .checked_add(size.checked_mul(index).context("program header overflow")?)
                .context("program header overflow")?;
            let header = elf
                .get(at..at.checked_add(56).context("program header overflow")?)
                .context("truncated program header")?;
            if u32::from_le_bytes(header[..4].try_into()?) != 1 {
                continue;
            }
            let file_offset = u64_at(header, 8)?;
            let virtual_address = u64_at(header, 16)?;
            if offset == file_offset & !0xfff {
                if let Some(address) = mapped
                    .checked_sub(virtual_address & !0xfff)
                    .and_then(|bias| bias.checked_add(target))
                {
                    addresses.insert(address);
                }
            }
        }
    }
    match addresses.into_iter().collect::<Vec<_>>().as_slice() {
        [address]
            if executable_ranges
                .iter()
                .any(|range| range.contains(address)) =>
        {
            Ok(*address)
        }
        _ => bail!("could not determine a unique ELF load bias"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lea(text: &mut [u8], at: usize, op: u8, target: i64) {
        text[at..at + 3].copy_from_slice(&[0x48, 0x8d, op]);
        text[at + 3..at + 7]
            .copy_from_slice(&((target - (0x1000 + at + 7) as i64) as i32).to_le_bytes());
    }
    #[test]
    fn two_hop_chain_handles_signed_displacements_and_ambiguity() {
        let mut data = vec![0x90; 0x700];
        data[0x20..0x23].copy_from_slice(&[0x55, 0x41, 0x57]);
        lea(&mut data, 0x30, 0x35, 0x800);
        lea(&mut data, 0x500, 0x3d, 0x800);
        lea(&mut data, 0x507, 0x35, 0x900);
        let rodata = Section {
            address: 0x900,
            data: b"com.Tencent.WCDB.Config.Cipher\0",
        };
        assert_eq!(
            locate_sections(
                Section {
                    address: 0x1000,
                    data: &data
                },
                rodata
            )
            .unwrap(),
            0x1020
        );
        data[0x600..0x603].copy_from_slice(&[0x55, 0x41, 0x57]);
        lea(&mut data, 0x620, 0x35, 0x800);
        assert!(locate_sections(
            Section {
                address: 0x1000,
                data: &data
            },
            rodata
        )
        .is_err());
        assert!(locate_elf(&[]).is_err());
        assert!(locate_elf(b"\x7fELF\x01\x01").is_err());
    }

    #[test]
    fn load_bias_handles_pie_and_fixed_address_executables() {
        let mut elf = vec![0u8; 120];
        elf[32..40].copy_from_slice(&64u64.to_le_bytes());
        elf[54..56].copy_from_slice(&56u16.to_le_bytes());
        elf[56..58].copy_from_slice(&1u16.to_le_bytes());
        elf[64..68].copy_from_slice(&1u32.to_le_bytes());
        for (virtual_address, mapped) in [(0u64, 0x700000u64), (0x400000, 0x400000)] {
            elf[80..88].copy_from_slice(&virtual_address.to_le_bytes());
            let maps = format!(
                "{mapped:x}-{:x} r-xp 00000000 00:01 1 /fixture\n",
                mapped + 0x4000
            );
            assert_eq!(
                runtime_address(&elf, &maps, virtual_address + 0x1234, "/fixture").unwrap(),
                mapped + 0x1234
            );
            assert!(runtime_address(
                &elf,
                &maps.replace("r-xp", "r--p"),
                virtual_address + 0x1234,
                "/fixture"
            )
            .is_err());
        }
        assert!(runtime_address(&[], "", 0, "/fixture").is_err());
    }
}
