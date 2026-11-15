//! Locating WeChat's database-key hook point in a 64-bit ELF image.
//!
//! The reference chain is the one described for `xkey_helper_linux`: find
//! `com.Tencent.WCDB.Config.Cipher` in `.rodata`, the `lea rsi` that points to
//! it (preceded by a `lea rdi` to the cipher configuration), the `lea rsi` that
//! points to that configuration, and then the nearest `55 41 57` function
//! prologue before it.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom};

use anyhow::{bail, Context as _, Result};

const ANCHOR: &[u8] = b"com.Tencent.WCDB.Config.Cipher";
const LEA_RSI: [u8; 3] = [0x48, 0x8d, 0x35];
const LEA_RDI: [u8; 3] = [0x48, 0x8d, 0x3d];
const PROLOGUE: [u8; 3] = [0x55, 0x41, 0x57];
const SEARCH_BACK: usize = 0x500;
const CHUNK: usize = 1024 * 1024;
const ELF_HEADER: usize = 64;
const SECTION_HEADER: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ImageKind {
    /// An executable linked at a fixed address.
    Executable,
    /// A position-independent executable; its sections' addresses are offsets
    /// from the load base.
    Dynamic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Section {
    pub address: usize,
    pub offset: usize,
    pub size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
    text: Section,
    rodata: Section,
    kind: ImageKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HookPoint {
    pub virtual_address: usize,
    pub kind: ImageKind,
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .context("truncated ELF header")?
            .try_into()?,
    ))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .context("truncated ELF header")?
            .try_into()?,
    ))
}

fn u64_at(bytes: &[u8], at: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .get(at..at + 8)
            .context("truncated ELF header")?
            .try_into()?,
    ))
}

fn read_at(file: &mut (impl Read + Seek), offset: u64, length: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0; length];
    file.seek(SeekFrom::Start(offset))
        .context("cannot seek in WeChat's ELF image")?;
    file.read_exact(&mut bytes)
        .context("truncated WeChat ELF image")?;
    Ok(bytes)
}

fn section(record: &[u8], file_size: u64, executable: bool) -> Result<Section> {
    let address = u64_at(record, 16)?;
    let offset = u64_at(record, 24)?;
    let size = u64_at(record, 32)?;
    let flags = u64_at(record, 8)?;
    if size == 0 {
        bail!("empty ELF section");
    }
    if executable && flags & 0x4 == 0 {
        bail!("ELF .text section is not executable");
    }
    if offset.checked_add(size).is_none_or(|end| end > file_size)
        || address.checked_add(size).is_none()
    {
        bail!("ELF section is outside WeChat's image");
    }
    Ok(Section {
        address: usize::try_from(address).context("ELF address does not fit")?,
        offset: usize::try_from(offset).context("ELF offset does not fit")?,
        size: usize::try_from(size).context("ELF section does not fit")?,
    })
}

fn layout(file: &mut (impl Read + Seek)) -> Result<Layout> {
    let file_size = file
        .seek(SeekFrom::End(0))
        .context("cannot measure WeChat's ELF image")?;
    let mut header = [0u8; ELF_HEADER];
    file.seek(SeekFrom::Start(0))
        .context("cannot rewind WeChat's ELF image")?;
    file.read_exact(&mut header)
        .context("truncated ELF header")?;
    if header.get(..4) != Some(b"\x7fELF") {
        bail!("WeChat is not an ELF image");
    }
    if header[4] != 2 || header[5] != 1 || header[6] != 1 {
        bail!("WeChat must be a 64-bit little-endian ELF image");
    }
    let kind = match u16_at(&header, 16)? {
        2 => ImageKind::Executable,
        3 => ImageKind::Dynamic,
        _ => bail!("unsupported ELF type"),
    };
    if u16_at(&header, 18)? != 62 {
        bail!("WeChat must be an x86-64 ELF image");
    }
    let table = u64_at(&header, 0x28)?;
    let entry_size = u16_at(&header, 0x3a)? as usize;
    if entry_size < SECTION_HEADER {
        bail!("invalid ELF section header size");
    }
    let mut count = u16_at(&header, 0x3c)? as usize;
    let mut names = u16_at(&header, 0x3e)? as usize;
    // ELF extended numbering stores the real counts in section 0.
    if count == 0 || names == 0xffff {
        let bytes = read_at(file, table, SECTION_HEADER)?;
        if count == 0 {
            count = usize::try_from(u64_at(&bytes, 32)?).context("too many ELF sections")?;
        }
        if names == 0xffff {
            names = u32_at(&bytes, 40)? as usize;
        }
    }
    if count == 0 || count > 0xffff {
        bail!("invalid ELF section count");
    }
    let table_end = table
        .checked_add(
            (count as u64)
                .checked_mul(entry_size as u64)
                .context("ELF table overflow")?,
        )
        .context("ELF table overflow")?;
    if table_end > file_size {
        bail!("truncated ELF section header table");
    }
    let mut headers = Vec::with_capacity(count);
    for index in 0..count {
        let offset = table + (index * entry_size) as u64;
        headers.push(read_at(file, offset, SECTION_HEADER)?);
    }
    let names_header = headers
        .get(names)
        .context("invalid ELF section name table")?;
    let names_offset = u64_at(names_header, 24)?;
    let names_size = u64_at(names_header, 32)?;
    if names_size == 0
        || names_offset
            .checked_add(names_size)
            .is_none_or(|end| end > file_size)
    {
        bail!("invalid ELF section name table");
    }
    let names_size = usize::try_from(names_size).context("ELF name table does not fit")?;
    let names = read_at(file, names_offset, names_size)?;
    let name_of = |header: &[u8]| -> Result<&[u8]> {
        let at = u32_at(header, 0)? as usize;
        let tail = names.get(at..).context("invalid ELF section name")?;
        let end = tail.iter().position(|b| *b == 0).unwrap_or(tail.len());
        Ok(&tail[..end])
    };
    let mut text = None;
    let mut rodata = None;
    for header in &headers[1..] {
        match name_of(header)? {
            b".text" => {
                if text.is_some() {
                    bail!("duplicate .text section in WeChat's ELF image");
                }
                text = Some(section(header, file_size, true)?);
            }
            b".rodata" => {
                if rodata.is_some() {
                    bail!("duplicate .rodata section in WeChat's ELF image");
                }
                rodata = Some(section(header, file_size, false)?);
            }
            _ => {}
        }
    }
    Ok(Layout {
        text: text.context("WeChat's ELF image has no .text section")?,
        rodata: rodata.context("WeChat's ELF image has no .rodata section")?,
        kind,
    })
}

/// Calls `visit(bytes, section_offset)` for overlapping chunks of `section`.
/// `lookback` bytes before each chunk are included, so a match that starts just
/// before a chunk boundary is seen with the bytes that precede it.
fn visit_section(
    file: &mut (impl Read + Seek),
    section: Section,
    lookback: usize,
    mut visit: impl FnMut(&[u8], usize) -> Result<()>,
) -> Result<()> {
    let mut offset = 0;
    while offset < section.size {
        let end = (offset + CHUNK).min(section.size);
        let start = offset.saturating_sub(lookback);
        let bytes = read_at(file, (section.offset + start) as u64, end - start)?;
        visit(&bytes, start)?;
        offset = end;
    }
    Ok(())
}

/// The address a `lea reg, [rip+disp32]` points to; `at` is the offset of its
/// opcode in `bytes` and `address` the address of the instruction.
fn lea_target(bytes: &[u8], at: usize, address: usize) -> Option<usize> {
    let disp = bytes.get(at + 3..at + 7)?;
    let disp = i32::from_le_bytes(disp.try_into().ok()?);
    Some(address.wrapping_add(7).wrapping_add_signed(disp as isize))
}

fn anchor_addresses(file: &mut (impl Read + Seek), section: Section) -> Result<Vec<usize>> {
    let mut found = BTreeSet::new();
    visit_section(file, section, ANCHOR.len() - 1, |bytes, start| {
        for (at, window) in bytes.windows(ANCHOR.len()).enumerate() {
            if window == ANCHOR {
                found.insert(section.address + start + at);
            }
        }
        Ok(())
    })?;
    Ok(found.into_iter().collect())
}

fn configuration_addresses(
    file: &mut (impl Read + Seek),
    section: Section,
    anchors: &[usize],
) -> Result<Vec<usize>> {
    let mut found = BTreeSet::new();
    visit_section(file, section, 7, |bytes, start| {
        for at in 0..bytes.len().saturating_sub(LEA_RSI.len() - 1) {
            if bytes[at..at + 3] != LEA_RSI {
                continue;
            }
            let instruction = section.address + start + at;
            let Some(target) = lea_target(bytes, at, instruction) else {
                continue;
            };
            if anchors.binary_search(&target).is_err() || at < 7 || bytes[at - 7..at - 4] != LEA_RDI
            {
                continue;
            }
            let unknown = lea_target(bytes, at - 7, instruction - 7);
            if let Some(unknown) = unknown {
                found.insert(unknown);
            }
        }
        Ok(())
    })?;
    Ok(found.into_iter().collect())
}

fn prologue_before(text: &[u8], at: usize) -> Option<usize> {
    let lower = at.saturating_sub(SEARCH_BACK);
    (lower..=at)
        .rev()
        .find(|back| text.get(*back..back + 3) == Some(&PROLOGUE[..]))
}

fn hook_addresses(
    file: &mut (impl Read + Seek),
    section: Section,
    configurations: &[usize],
) -> Result<Vec<usize>> {
    let mut candidates = BTreeMap::new();
    visit_section(file, section, SEARCH_BACK + 7, |bytes, start| {
        for at in 0..bytes.len().saturating_sub(LEA_RSI.len() - 1) {
            if bytes[at..at + 3] != LEA_RSI {
                continue;
            }
            let instruction = section.address + start + at;
            let Some(target) = lea_target(bytes, at, instruction) else {
                continue;
            };
            if configurations.binary_search(&target).is_err() {
                continue;
            }
            if let Some(back) = prologue_before(bytes, at) {
                candidates
                    .entry(instruction)
                    .or_insert(section.address + start + back);
            }
        }
        Ok(())
    })?;
    Ok(candidates.into_values().collect())
}

/// The virtual address of the function to hook, before relocation.
pub(super) fn locate(file: &mut (impl Read + Seek)) -> Result<HookPoint> {
    let layout = layout(file)?;
    let anchors = anchor_addresses(file, layout.rodata)?;
    if anchors.is_empty() {
        bail!("the WeChat database-key string was not found in .rodata");
    }
    let configurations = configuration_addresses(file, layout.text, &anchors)?;
    if configurations.is_empty() {
        bail!("no reference to the database-key configuration was found in .text");
    }
    let hooks = hook_addresses(file, layout.text, &configurations)?;
    let virtual_address = *hooks
        .first()
        .context("the WeChat database-key function prologue was not found")?;
    Ok(HookPoint {
        virtual_address,
        kind: layout.kind,
    })
}

#[cfg(test)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the synthetic ELF fixture values are small"
)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const TEXT_ADDRESS: usize = 0x2000;
    const RODATA_ADDRESS: usize = 0x3000;

    fn put_u16(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn lea(bytes: &mut [u8], at: usize, opcode: [u8; 3], target: usize, address: usize) {
        bytes[at..at + 3].copy_from_slice(&opcode);
        let next = address + 7;
        let disp = i64::try_from(target).unwrap() - i64::try_from(next).unwrap();
        bytes[at + 3..at + 7].copy_from_slice(&i32::try_from(disp).unwrap().to_le_bytes());
    }

    fn elf(kind: u16, text_address: usize, text: &[u8], rodata: &[u8]) -> Vec<u8> {
        let names = b"\0.text\0.rodata\0.shstrtab\0";
        let rodata_offset = 0x100;
        let text_offset = rodata_offset + rodata.len();
        let names_offset = text_offset + text.len();
        let table_offset = names_offset + names.len();
        let mut bytes = vec![0u8; table_offset + 4 * SECTION_HEADER];
        bytes[..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[6] = 1;
        put_u16(&mut bytes, 16, kind);
        put_u16(&mut bytes, 18, 62);
        put_u32(&mut bytes, 20, 1);
        put_u64(&mut bytes, 0x28, table_offset as u64);
        put_u16(&mut bytes, 0x34, ELF_HEADER as u16);
        put_u16(&mut bytes, 0x3a, SECTION_HEADER as u16);
        put_u16(&mut bytes, 0x3c, 4);
        put_u16(&mut bytes, 0x3e, 3);
        bytes[rodata_offset..rodata_offset + rodata.len()].copy_from_slice(rodata);
        bytes[text_offset..text_offset + text.len()].copy_from_slice(text);
        bytes[names_offset..names_offset + names.len()].copy_from_slice(names);
        let headers = [
            [0u32, 0, 0, 0, 0],
            [
                1,
                0x6,
                text_address as u32,
                text_offset as u32,
                text.len() as u32,
            ],
            [
                7,
                0x2,
                RODATA_ADDRESS as u32,
                rodata_offset as u32,
                rodata.len() as u32,
            ],
            [15, 0x0, 0, names_offset as u32, names.len() as u32],
        ];
        for (index, header) in headers.iter().enumerate() {
            let at = table_offset + index * SECTION_HEADER;
            put_u32(&mut bytes, at, header[0]);
            put_u32(&mut bytes, at + 4, if index == 3 { 3 } else { 1 });
            put_u64(&mut bytes, at + 8, header[1] as u64);
            put_u64(&mut bytes, at + 16, header[2] as u64);
            put_u64(&mut bytes, at + 24, header[3] as u64);
            put_u64(&mut bytes, at + 32, header[4] as u64);
        }
        bytes
    }

    /// A well-formed reference chain: the first `lea rsi` refers to the
    /// string and follows a `lea rdi`; the second one refers to the
    /// configuration and has a prologue at `prologue`.
    fn image(prologue: usize, second_hop: usize) -> Vec<u8> {
        let mut text = vec![0u8; 0x400];
        let unknown = TEXT_ADDRESS + 0x300;
        let anchor = RODATA_ADDRESS;
        text[prologue..prologue + 3].copy_from_slice(&PROLOGUE);
        lea(&mut text, 0xf9, LEA_RDI, unknown, TEXT_ADDRESS + 0xf9);
        lea(&mut text, 0x100, LEA_RSI, anchor, TEXT_ADDRESS + 0x100);
        lea(
            &mut text,
            second_hop,
            LEA_RSI,
            unknown,
            TEXT_ADDRESS + second_hop,
        );
        let mut rodata = vec![0u8; 0x20];
        rodata[..ANCHOR.len()].copy_from_slice(ANCHOR);
        elf(3, TEXT_ADDRESS, &text, &rodata)
    }

    #[test]
    fn locates_the_synthetic_hook_point() {
        let bytes = image(0x1d0, 0x200);
        let point = locate(&mut Cursor::new(bytes)).unwrap();
        assert_eq!(
            point,
            HookPoint {
                virtual_address: TEXT_ADDRESS + 0x1d0,
                kind: ImageKind::Dynamic,
            }
        );
    }

    #[test]
    fn the_first_second_hop_wins() {
        let mut text = vec![0u8; 0x400];
        let unknown = TEXT_ADDRESS + 0x300;
        text[0x1f0..0x1f3].copy_from_slice(&PROLOGUE);
        text[0x3d0..0x3d3].copy_from_slice(&PROLOGUE);
        lea(&mut text, 0xf9, LEA_RDI, unknown, TEXT_ADDRESS + 0xf9);
        lea(
            &mut text,
            0x100,
            LEA_RSI,
            RODATA_ADDRESS,
            TEXT_ADDRESS + 0x100,
        );
        lea(&mut text, 0x200, LEA_RSI, unknown, TEXT_ADDRESS + 0x200);
        lea(&mut text, 0x3e0, LEA_RSI, unknown, TEXT_ADDRESS + 0x3e0);
        let mut rodata = vec![0u8; 0x20];
        rodata[..ANCHOR.len()].copy_from_slice(ANCHOR);
        let bytes = elf(3, TEXT_ADDRESS, &text, &rodata);
        assert_eq!(
            locate(&mut Cursor::new(bytes)).unwrap().virtual_address,
            TEXT_ADDRESS + 0x1f0
        );
    }

    #[test]
    fn requires_the_lea_rdi_before_the_first_hop() {
        let mut text = vec![0u8; 0x400];
        text[0x1d0..0x1d3].copy_from_slice(&PROLOGUE);
        lea(
            &mut text,
            0x100,
            LEA_RSI,
            RODATA_ADDRESS,
            TEXT_ADDRESS + 0x100,
        );
        lea(
            &mut text,
            0x200,
            LEA_RSI,
            TEXT_ADDRESS + 0x300,
            TEXT_ADDRESS + 0x200,
        );
        let mut rodata = vec![0u8; 0x20];
        rodata[..ANCHOR.len()].copy_from_slice(ANCHOR);
        let bytes = elf(3, TEXT_ADDRESS, &text, &rodata);
        assert!(locate(&mut Cursor::new(bytes)).is_err());
    }

    #[test]
    fn reports_missing_anchors_and_references() {
        let rodata = vec![0u8; 0x20];
        let text = vec![0u8; 0x20];
        let bytes = elf(3, TEXT_ADDRESS, &text, &rodata);
        assert!(locate(&mut Cursor::new(bytes)).is_err());
        let mut rodata = vec![0u8; 0x20];
        rodata[..ANCHOR.len()].copy_from_slice(ANCHOR);
        let bytes = elf(3, TEXT_ADDRESS, &text, &rodata);
        assert!(locate(&mut Cursor::new(bytes)).is_err());
    }

    #[test]
    fn requires_a_near_prologue() {
        let mut text = vec![0u8; 0x800];
        let unknown = TEXT_ADDRESS + 0x300;
        lea(&mut text, 0xf9, LEA_RDI, unknown, TEXT_ADDRESS + 0xf9);
        lea(
            &mut text,
            0x100,
            LEA_RSI,
            RODATA_ADDRESS,
            TEXT_ADDRESS + 0x100,
        );
        lea(&mut text, 0x700, LEA_RSI, unknown, TEXT_ADDRESS + 0x700);
        let mut rodata = vec![0u8; 0x20];
        rodata[..ANCHOR.len()].copy_from_slice(ANCHOR);
        let bytes = elf(3, TEXT_ADDRESS, &text, &rodata);
        assert!(locate(&mut Cursor::new(bytes)).is_err());
    }

    #[test]
    fn rejects_invalid_elf_images() {
        let bytes = image(0x1d0, 0x200);
        assert!(locate(&mut Cursor::new(&bytes[..ELF_HEADER - 1])).is_err());
        for at in [4, 5, 6] {
            let mut changed = bytes.clone();
            changed[at] ^= 1;
            assert!(locate(&mut Cursor::new(changed)).is_err());
        }
        let mut changed = bytes.clone();
        put_u16(&mut changed, 18, 3);
        assert!(locate(&mut Cursor::new(changed)).is_err());
        let mut changed = bytes.clone();
        put_u16(&mut changed, 16, 1);
        assert!(locate(&mut Cursor::new(changed)).is_err());
        let mut changed = bytes.clone();
        put_u16(&mut changed, 0x3c, 0);
        assert!(locate(&mut Cursor::new(changed)).is_err());
        let mut changed = bytes;
        put_u64(&mut changed, 0x28, u64::MAX);
        assert!(locate(&mut Cursor::new(changed)).is_err());
    }

    #[test]
    fn supports_fixed_address_executables() {
        let bytes = image(0x1d0, 0x200);
        let mut changed = bytes;
        put_u16(&mut changed, 16, 2);
        let point = locate(&mut Cursor::new(changed)).unwrap();
        assert_eq!(point.kind, ImageKind::Executable);
        assert_eq!(point.virtual_address, TEXT_ADDRESS + 0x1d0);
    }
}
