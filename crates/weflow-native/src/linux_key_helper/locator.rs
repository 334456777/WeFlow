//! The ELF scanner of `xkey_helper_linux`, ported step for step.
//!
//! It reads the whole target file, uses 64-byte section headers and the
//! section-name table, finds `com.Tencent.WCDB.Config.Cipher` in `.rodata`,
//! follows the two `lea rsi` references and the `lea rdi` between them, then
//! scans back for the `55 41 57` function prologue. The first match of each
//! kind wins, exactly as in the original binary.

const ANCHOR: &[u8] = b"com.Tencent.WCDB.Config.Cipher";
const LEA_RSI: [u8; 3] = [0x48, 0x8d, 0x35];
const LEA_RDI: [u8; 3] = [0x48, 0x8d, 0x3d];
const PROLOGUE: [u8; 3] = [0x55, 0x41, 0x57];
const SEARCH_BACK: usize = 0x500;
const SECTION_HEADER: usize = 0x40;

const MISSING_SECTIONS: &str = "Failed to find .rodata or .text section.";
const NO_STRING: &str = "String not found in .rodata.";
const NO_XREF: &str = "String xref (lea rsi) not found in .text.";
const NO_LEA_RDI: &str = "Failed to locate lea rdi before string xref.";
const NO_UNKNOWN: &str = "No lea rsi, unk_VA found.";
const NO_PROLOGUE: &str = "Failed to find function prologue.";

#[derive(Debug, Clone, Copy)]
struct Section {
    address: usize,
    offset: usize,
    size: usize,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn section(bytes: &[u8], header: usize) -> Option<Section> {
    let address = u64_at(bytes, header + 0x10)?;
    let offset = u64_at(bytes, header + 0x18)?;
    let size = u64_at(bytes, header + 0x20)?;
    let end = offset.checked_add(size)?;
    if size == 0 || end > bytes.len() as u64 {
        return None;
    }
    Some(Section {
        address: usize::try_from(address).ok()?,
        offset: usize::try_from(offset).ok()?,
        size: usize::try_from(size).ok()?,
    })
}

fn find_sections(bytes: &[u8]) -> Result<(Section, Section), &'static str> {
    // 0x7f 'E' 'L' 'F' in the first byte and the ELFCLASS64 / ELFDATA2LSB
    // bytes, as checked by the original scanner.
    if bytes.len() < 8 || bytes[0] != 0x7f || bytes[4] != 2 || bytes[5] != 1 {
        return Err(MISSING_SECTIONS);
    }
    let table = u64_at(bytes, 0x28).ok_or(MISSING_SECTIONS)?;
    let count = u16_at(bytes, 0x3c).ok_or(MISSING_SECTIONS)? as usize;
    let names = u16_at(bytes, 0x3e).ok_or(MISSING_SECTIONS)? as usize;
    if count == 0 {
        return Err(MISSING_SECTIONS);
    }
    let table = usize::try_from(table).map_err(|_| MISSING_SECTIONS)?;
    let names_header = table
        .checked_add(names.checked_mul(SECTION_HEADER).ok_or(MISSING_SECTIONS)?)
        .ok_or(MISSING_SECTIONS)?;
    let names_offset = u64_at(bytes, names_header + 0x18).ok_or(MISSING_SECTIONS)?;
    let names_offset = usize::try_from(names_offset).map_err(|_| MISSING_SECTIONS)?;
    let mut rodata = None;
    let mut text = None;
    for index in 0..count {
        let header = table
            .checked_add(index.checked_mul(SECTION_HEADER).ok_or(MISSING_SECTIONS)?)
            .ok_or(MISSING_SECTIONS)?;
        let name_at = u32_at(bytes, header).ok_or(MISSING_SECTIONS)? as usize;
        let name_at = names_offset.checked_add(name_at).ok_or(MISSING_SECTIONS)?;
        let tail = bytes.get(name_at..).ok_or(MISSING_SECTIONS)?;
        let end = tail
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(tail.len());
        let name = &tail[..end];
        if name == b".rodata" && rodata.is_none() {
            rodata = Some(section(bytes, header).ok_or(MISSING_SECTIONS)?);
        } else if name == b".text" && text.is_none() {
            text = Some(section(bytes, header).ok_or(MISSING_SECTIONS)?);
        }
    }
    Ok((
        rodata.ok_or(MISSING_SECTIONS)?,
        text.ok_or(MISSING_SECTIONS)?,
    ))
}

fn anchor(bytes: &[u8], rodata: Section) -> Result<usize, &'static str> {
    let data = &bytes[rodata.offset..rodata.offset + rodata.size];
    let at = data
        .windows(ANCHOR.len())
        .position(|window| window == ANCHOR)
        .ok_or(NO_STRING)?;
    let address = rodata.address.checked_add(at).ok_or(NO_STRING)?;
    if address == 0 {
        Err(NO_STRING)
    } else {
        Ok(address)
    }
}

/// The address of the first `lea rsi` in `.text` whose target is `target`.
fn first_lea_rsi(bytes: &[u8], text: Section, target: usize) -> Option<usize> {
    let data = &bytes[text.offset..text.offset + text.size];
    let last = data.len().checked_sub(7)?;
    for at in 0..=last {
        if data[at..at + 3] != LEA_RSI {
            continue;
        }
        let disp = i32::from_le_bytes(data[at + 3..at + 7].try_into().ok()?);
        let instruction = text.address.wrapping_add(at);
        if instruction
            .wrapping_add(7)
            .wrapping_add_signed(disp as isize)
            == target
        {
            return Some(at);
        }
    }
    None
}

fn prologue(bytes: &[u8], text: Section, offset: usize) -> Option<usize> {
    let data = &bytes[text.offset..text.offset + text.size];
    let lower = if offset > SEARCH_BACK {
        offset - SEARCH_BACK + 1
    } else {
        1
    };
    for at in (lower..=offset).rev() {
        if data.get(at..at + 3) == Some(&PROLOGUE[..]) {
            return text.address.checked_add(at);
        }
    }
    None
}

/// Locates the hook point in an ELF image; the returned address is the
/// section-relative virtual address before the process load base is added.
pub(super) fn locate(bytes: &[u8]) -> Result<usize, &'static str> {
    let (rodata, text) = find_sections(bytes)?;
    let anchor = anchor(bytes, rodata)?;
    let first = first_lea_rsi(bytes, text, anchor).ok_or(NO_XREF)?;
    if first < 7 {
        return Err(NO_LEA_RDI);
    }
    let data = &bytes[text.offset..text.offset + text.size];
    if data[first - 7..first - 4] != LEA_RDI {
        return Err(NO_LEA_RDI);
    }
    let unknown = {
        let disp = i32::from_le_bytes(data[first - 4..first].try_into().map_err(|_| NO_LEA_RDI)?);
        text.address
            .wrapping_add(first)
            .wrapping_add_signed(disp as isize)
    };
    let second = first_lea_rsi(bytes, text, unknown).ok_or(NO_UNKNOWN)?;
    let instruction = text.address.wrapping_add(second);
    if instruction < text.address
        || instruction > text.address.wrapping_add(text.size)
        || instruction == text.address
    {
        return Err(NO_PROLOGUE);
    }
    prologue(bytes, text, second)
        .filter(|address| *address != 0)
        .ok_or(NO_PROLOGUE)
}

#[cfg(test)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the synthetic ELF fixture values are small"
)]
mod tests {
    use super::*;

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

    fn elf(text_address: usize, text: &[u8], rodata: &[u8]) -> Vec<u8> {
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
        put_u16(&mut bytes, 16, 3);
        put_u16(&mut bytes, 18, 62);
        put_u32(&mut bytes, 20, 1);
        put_u64(&mut bytes, 0x28, table_offset as u64);
        put_u16(&mut bytes, 0x34, 0x40);
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

    fn image(prologue: usize, second_hop: usize) -> Vec<u8> {
        let mut text = vec![0u8; 0x400];
        let unknown = TEXT_ADDRESS + 0x300;
        text[prologue..prologue + 3].copy_from_slice(&PROLOGUE);
        lea(&mut text, 0xf9, LEA_RDI, unknown, TEXT_ADDRESS + 0xf9);
        lea(
            &mut text,
            0x100,
            LEA_RSI,
            RODATA_ADDRESS,
            TEXT_ADDRESS + 0x100,
        );
        lea(
            &mut text,
            second_hop,
            LEA_RSI,
            unknown,
            TEXT_ADDRESS + second_hop,
        );
        let mut rodata = vec![0u8; 0x20];
        rodata[..ANCHOR.len()].copy_from_slice(ANCHOR);
        elf(TEXT_ADDRESS, &text, &rodata)
    }

    #[test]
    fn locates_the_synthetic_hook_point() {
        assert_eq!(locate(&image(0x1d0, 0x200)).unwrap(), TEXT_ADDRESS + 0x1d0);
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
        assert_eq!(
            locate(&elf(TEXT_ADDRESS, &text, &rodata)).unwrap(),
            TEXT_ADDRESS + 0x1f0
        );
    }

    #[test]
    fn reports_the_same_errors_as_the_helper() {
        let rodata = vec![0u8; 0x20];
        let text = vec![0u8; 0x20];
        assert_eq!(locate(&elf(TEXT_ADDRESS, &text, &rodata)), Err(NO_STRING));
        let mut rodata = vec![0u8; 0x20];
        rodata[..ANCHOR.len()].copy_from_slice(ANCHOR);
        assert_eq!(locate(&elf(TEXT_ADDRESS, &text, &rodata)), Err(NO_XREF));
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
        assert_eq!(locate(&elf(TEXT_ADDRESS, &text, &rodata)), Err(NO_LEA_RDI));
        let mut text = vec![0u8; 0x400];
        lea(
            &mut text,
            0x100,
            LEA_RSI,
            RODATA_ADDRESS,
            TEXT_ADDRESS + 0x100,
        );
        assert_eq!(locate(&elf(TEXT_ADDRESS, &text, &rodata)), Err(NO_LEA_RDI));
        let mut text = vec![0u8; 0x400];
        lea(
            &mut text,
            0xf9,
            LEA_RDI,
            TEXT_ADDRESS + 0x300,
            TEXT_ADDRESS + 0xf9,
        );
        lea(
            &mut text,
            0x100,
            LEA_RSI,
            RODATA_ADDRESS,
            TEXT_ADDRESS + 0x100,
        );
        assert_eq!(locate(&elf(TEXT_ADDRESS, &text, &rodata)), Err(NO_UNKNOWN));
        let mut text = vec![0u8; 0x800];
        lea(
            &mut text,
            0xf9,
            LEA_RDI,
            TEXT_ADDRESS + 0x300,
            TEXT_ADDRESS + 0xf9,
        );
        lea(
            &mut text,
            0x100,
            LEA_RSI,
            RODATA_ADDRESS,
            TEXT_ADDRESS + 0x100,
        );
        lea(
            &mut text,
            0x700,
            LEA_RSI,
            TEXT_ADDRESS + 0x300,
            TEXT_ADDRESS + 0x700,
        );
        assert_eq!(locate(&elf(TEXT_ADDRESS, &text, &rodata)), Err(NO_PROLOGUE));
        let bytes = image(0x1d0, 0x200);
        assert_eq!(locate(&bytes[..64]), Err(MISSING_SECTIONS));
        let mut changed = bytes;
        changed[4] = 1;
        assert_eq!(locate(&changed), Err(MISSING_SECTIONS));
    }
}
