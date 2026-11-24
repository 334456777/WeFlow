//! The ARM64 scanner of `xkey_helper`, ported instruction for instruction.
//!
//! The original looked for the anchor `mov x3, x2; mov x2, x1`, stepped back
//! `0x10` bytes and accepted the candidate only when the six instructions and
//! a nearby unconditional branch had the expected shape. A second, stricter
//! check read the branch target (the database sink). This module keeps those
//! pure byte checks; `platform` supplies the memory.

/// `mov x3, x2; mov x2, x1`, as the original searched for it.
pub(super) const ANCHOR: [u8; 8] = [0xe3, 0x03, 0x02, 0xaa, 0xe2, 0x03, 0x01, 0xaa];
const ANCHOR_BACK: u64 = 0x10;
const WINDOW: usize = 0x80;
const SINK_WINDOW: usize = 0x20;

fn word(bytes: &[u8], index: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(index * 4..index * 4 + 4)?.try_into().ok()?,
    ))
}

fn register(value: u32) -> u32 {
    value & 0x1f
}

/// The six instructions the original required at `candidate`: `ADRP`, `LDR`,
/// `CBZ`, `BR`, `mov x3, x2`, `mov x2, x1`, with one register throughout, and
/// an unconditional `B` in the following `0x80` bytes.
pub(super) fn initial_shape(bytes: &[u8]) -> bool {
    if bytes.len() < WINDOW {
        return false;
    }
    let Some(adrp) = word(bytes, 0) else {
        return false;
    };
    let Some(ldr) = word(bytes, 1) else {
        return false;
    };
    let Some(cbz) = word(bytes, 2) else {
        return false;
    };
    let Some(br) = word(bytes, 3) else {
        return false;
    };
    if word(bytes, 4) != Some(0xaa02_03e3) || word(bytes, 5) != Some(0xaa01_03e2) {
        return false;
    }
    let key = register(adrp);
    if adrp & 0x9f00_0000 != 0x9000_0000
        || ldr & 0xffc0_0000 != 0xf940_0000
        || cbz & 0x7f00_0000 != 0x3400_0000
        || br & 0xffff_fc1f != 0xd61f_0000
        || register(ldr) != key
        || (ldr >> 5) & 0x1f != key
        || cbz & 0x8000_0000 == 0
        || register(cbz) != key
        || (br >> 5) & 0x1f != key
    {
        return false;
    }
    has_unconditional_branch(bytes)
}

fn has_unconditional_branch(bytes: &[u8]) -> bool {
    (0..WINDOW / 4)
        .any(|index| word(bytes, index).is_some_and(|word| word & 0xfc00_0000 == 0x1400_0000))
}

/// The database sink: `ADRP`, `LDR`, two 64-bit `CBZ`s and `cbz x0`,
/// `cbz x2`, `cbz w3`, with the ADRP/LDR/CBZ registers agreeing.
pub(super) fn strict_sink(bytes: &[u8]) -> bool {
    if bytes.len() < SINK_WINDOW {
        return false;
    }
    let Some(adrp) = word(bytes, 0) else {
        return false;
    };
    let Some(ldr) = word(bytes, 1) else {
        return false;
    };
    let Some(first_cbz) = word(bytes, 2) else {
        return false;
    };
    let Some(second_cbz) = word(bytes, 3) else {
        return false;
    };
    let Some(x0_cbz) = word(bytes, 4) else {
        return false;
    };
    let Some(x2_cbz) = word(bytes, 5) else {
        return false;
    };
    let Some(w3_cbz) = word(bytes, 6) else {
        return false;
    };
    let Some(_unused) = word(bytes, 7) else {
        return false;
    };
    let key = register(adrp);
    if adrp & 0x9f00_0000 != 0x9000_0000
        || ldr & 0xffc0_0000 != 0xf940_0000
        || first_cbz & 0x7f00_0000 != 0x3400_0000
        || second_cbz & 0x7f00_0000 != 0x3400_0000
        || x0_cbz & 0x7f00_0000 != 0x3400_0000
        || x2_cbz & 0x7f00_0000 != 0x3400_0000
        || w3_cbz & 0x7f00_0000 != 0x3400_0000
        || register(ldr) != key
        || (ldr >> 5) & 0x1f != key
        || first_cbz & 0x8000_0000 == 0
        || register(first_cbz) != key
        || (second_cbz >> 5) & 0x1f != key
        || x0_cbz & 0x8000_001f != 0x8000_0000
        || x2_cbz & 0x8000_001f != 0x8000_0002
        || w3_cbz & 0x8000_0000 != 0
        || register(w3_cbz) != 3
    {
        return false;
    }
    true
}

/// The next instruction to run when the candidate is `address`, for each
/// unconditional `B` in its `0x80`-byte window.
pub(super) fn branch_targets(bytes: &[u8], address: u64) -> Vec<u64> {
    let mut targets = Vec::new();
    for index in 0..WINDOW / 4 {
        let Some(word) = word(bytes, index) else {
            break;
        };
        if word & 0xfc00_0000 != 0x1400_0000 {
            continue;
        }
        let immediate = word & 0x03ff_ffff;
        let offset = i64::from((immediate.cast_signed() << 6) >> 6);
        targets.push(address.wrapping_add_signed(offset * 4));
    }
    targets
}

/// The anchor offsets in `bytes`; the caller adds the region base.
pub(super) fn anchor_offsets(bytes: &[u8]) -> Vec<usize> {
    bytes
        .windows(ANCHOR.len())
        .enumerate()
        .filter_map(|(index, window)| (window == ANCHOR).then_some(index))
        .collect()
}

/// The candidate address of an anchor at `anchor`: `anchor - 0x10`.
pub(super) fn candidate(anchor: u64) -> Option<u64> {
    anchor.checked_sub(ANCHOR_BACK)
}

/// The strict candidate closest to `module_base + known_rva`, as the original
/// chose it.
pub(super) fn pick(candidates: &[u64], module_base: u64, known_rva: u64) -> Option<u64> {
    let wanted = module_base.wrapping_add(known_rva);
    candidates
        .iter()
        .copied()
        .min_by_key(|address| address.abs_diff(wanted))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADRP_X0: u32 = 0x9000_0000;
    const LDR_X0_X0: u32 = 0xf940_0000;
    const CBZ_X0: u32 = 0xb400_0000;
    const BR_X0: u32 = 0xd61f_0000;
    const MOV_X3_X2: u32 = 0xaa02_03e3;
    const MOV_X2_X1: u32 = 0xaa01_03e2;

    fn put(word: u32, bytes: &mut Vec<u8>) {
        let at = bytes.len();
        bytes.extend_from_slice(&word.to_le_bytes());
        assert_eq!(at % 4, 0);
    }

    fn initial() -> Vec<u8> {
        let mut bytes = Vec::new();
        for word in [ADRP_X0, LDR_X0_X0, CBZ_X0, BR_X0, MOV_X3_X2, MOV_X2_X1] {
            put(word, &mut bytes);
        }
        put(0x1400_0001, &mut bytes); // b +4
        while bytes.len() < WINDOW {
            put(0xd503_201f, &mut bytes); // nop
        }
        bytes
    }

    fn sink() -> Vec<u8> {
        let mut bytes = Vec::new();
        for word in [
            ADRP_X0,
            LDR_X0_X0,
            CBZ_X0,
            CBZ_X0,
            0xb400_0000, // cbz x0
            0xb400_0002, // cbz x2
            0x3400_0003, // cbz w3
            0xd503_201f, // nop
        ] {
            put(word, &mut bytes);
        }
        bytes
    }

    #[test]
    fn accepts_the_shape_the_original_accepted() {
        assert!(initial_shape(&initial()));
        let mut broken = initial();
        broken[3] ^= 0x80;
        assert!(!initial_shape(&broken));
        let mut broken = initial();
        broken[16] = 0;
        assert!(!initial_shape(&broken));
    }

    #[test]
    fn requires_a_branch_in_the_window() {
        let mut bytes = initial();
        bytes[24..28].copy_from_slice(&0xd503_201fu32.to_le_bytes());
        assert!(!initial_shape(&bytes));
        bytes[24..28].copy_from_slice(&(0x1400_0000u32).to_le_bytes());
        assert!(initial_shape(&bytes));
    }

    #[test]
    fn accepts_the_sink_the_original_accepted() {
        assert!(strict_sink(&sink()));
        for index in [0, 1, 2, 4, 5, 6] {
            let mut broken = sink();
            broken[index * 4] ^= 0x01;
            assert!(!strict_sink(&broken), "word {index}");
        }
        let mut broken = sink();
        broken[12..16].copy_from_slice(&0xb400_0020u32.to_le_bytes());
        assert!(!strict_sink(&broken), "second cbz register");
    }

    #[test]
    fn decodes_positive_and_negative_branch_offsets() {
        let mut bytes = vec![0; WINDOW];
        bytes[0..4].copy_from_slice(&(0x1400_0001u32).to_le_bytes());
        bytes[4..8].copy_from_slice(&(0x17ff_ffffu32).to_le_bytes());
        assert_eq!(
            branch_targets(&bytes, 0x1000),
            vec![0x1004, 0x1000_u64.wrapping_sub(4)]
        );
    }

    #[test]
    fn finds_anchors_and_steps_back_sixteen_bytes() {
        let mut bytes = vec![0u8; 0x40];
        bytes[0x18..0x20].copy_from_slice(&ANCHOR);
        assert_eq!(anchor_offsets(&bytes), vec![0x18]);
        assert_eq!(candidate(0x1018), Some(0x1008));
        assert_eq!(candidate(0x8), None);
    }

    #[test]
    fn picks_the_candidate_closest_to_the_known_offset() {
        let candidates = [0x1000, 0x2200, 0x3100];
        assert_eq!(pick(&candidates, 0x1000, 0x1000), Some(0x2200));
        assert_eq!(pick(&candidates, 0x1000, 0), Some(0x1000));
        assert_eq!(pick(&[], 0x1000, 0), None);
    }
}
