//! Files: pictures, videos, voices and their encrypted `.dat` form.

use aes::cipher::{BlockCipherEncrypt, KeyInit};

use super::rng::Rng;
use super::{DAY, IMAGE_AES_KEY, IMAGE_XOR_KEY, TZ};

/// A V2 `.dat` file: the head AES-encrypted with [`IMAGE_AES_KEY`], the tail XOR-ed with [`IMAGE_XOR_KEY`].
pub(super) fn encrypt_dat(plain: &[u8]) -> Vec<u8> {
    let aes_len = plain.len().min(1024);
    let xor_len = (plain.len() - aes_len).min(100);
    let mut head = plain[..aes_len].to_vec();
    let pad = 16 - head.len() % 16;
    head.extend(std::iter::repeat_n(pad as u8, pad));
    let mut key = [0u8; 16];
    key.copy_from_slice(&IMAGE_AES_KEY.as_bytes()[..16]);
    let cipher = aes::Aes128::new(&key.into());
    let mut out = vec![0x07, 0x08, 0x56, 0x32, 0x08, 0x07];
    out.extend((aes_len as i32).to_le_bytes());
    out.extend((xor_len as i32).to_le_bytes());
    out.push(1);
    for block in head.as_chunks_mut::<16>().0 {
        let mut b = aes::cipher::Array::from(*block);
        cipher.encrypt_block(&mut b);
        out.extend_from_slice(&b);
    }
    out.extend_from_slice(&plain[aes_len..plain.len() - xor_len]);
    out.extend(
        plain[plain.len() - xor_len..]
            .iter()
            .map(|b| b ^ IMAGE_XOR_KEY),
    );
    out
}

/// A JPEG of `len` bytes: a JFIF header, comment segments of noise and the end marker (not a picture).
pub(super) fn jpeg(rng: &mut Rng, len: usize) -> Vec<u8> {
    let mut out = vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0,
    ];
    while out.len() + 4 + 2 < len {
        let n = (len - out.len() - 6).clamp(1, 65_533);
        out.extend([0xFF, 0xFE]);
        out.extend(((n + 2) as u16).to_be_bytes());
        out.extend(rng.bytes(n));
    }
    out.extend([0xFF, 0xD9]);
    out
}

fn crc32(chunks: &[&[u8]]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, entry) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *entry = c;
        }
        t
    });
    let mut crc = !0u32;
    for b in chunks.iter().flat_map(|c| c.iter()) {
        crc = table[((crc ^ u32::from(*b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(kind);
    out.extend(data);
    out.extend(crc32(&[kind, data]).to_be_bytes());
}

/// A valid PNG of one grey pixel padded to `len` bytes with a private chunk of noise.
pub(super) fn png(rng: &mut Rng, len: usize) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png_chunk(&mut out, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]);
    // zlib, one stored block of the scanline (filter 0, grey 0x80), and its Adler-32
    png_chunk(
        &mut out,
        b"IDAT",
        &[
            0x78, 0x01, 0x01, 0x02, 0x00, 0xFD, 0xFF, 0x00, 0x80, 0x00, 0x83, 0x00, 0x81,
        ],
    );
    let pad = len.saturating_sub(out.len() + 12 + 12);
    if pad > 0 {
        png_chunk(&mut out, b"wfPd", &rng.bytes(pad));
    }
    png_chunk(&mut out, b"IEND", &[]);
    out
}

/// A WXGF picture the built-in decoder reads (a small HEVC frame), padded to `len` bytes with filler NAL units.
pub(super) fn wxgf(len: usize) -> Vec<u8> {
    const HEVC: &[u8] = include_bytes!("../../../../weflow-core/testdata/wxgf/full_range.hevc");
    let mut out = b"wxgf".to_vec();
    out.extend([0x24, 0x01, 0x00, 0x46, 0x00, 0x2e]);
    out.extend(HEVC);
    while out.len() + 6 < len {
        // filler data (NAL type 38): 0xFF bytes up to the stop bit
        let n = (len - out.len() - 6).min(60_000);
        out.extend([0x00, 0x00, 0x01, 0x4C, 0x01]);
        out.extend(std::iter::repeat_n(0xFF, n));
        out.push(0x80);
    }
    out
}

/// An MP4 of `len` bytes: a file type box and a media data box of noise.
pub(super) fn mp4(rng: &mut Rng, len: usize) -> Vec<u8> {
    let mut out = vec![
        0, 0, 0, 0x18, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm', 0, 0, 2, 0, b'i', b's',
        b'o', b'm', b'i', b's', b'o', b'2',
    ];
    let data = len.saturating_sub(out.len() + 8);
    out.extend(((data + 8) as u32).to_be_bytes());
    out.extend(b"mdat");
    out.extend(rng.bytes(data));
    out
}

/// The packets of a second of a 440 Hz tone the SILK SDK encoded at 24 kHz (20 ms each).
static TONE_PACKETS: std::sync::LazyLock<Vec<Vec<u8>>> = std::sync::LazyLock::new(|| {
    let stream = weflow_silk::testenc::encode_tone(24_000);
    let mut packets = Vec::new();
    let mut rest = &stream[b"#!SILK_V3".len()..];
    while let [lo, hi, tail @ ..] = rest {
        let Ok(len) = usize::try_from(i16::from_le_bytes([*lo, *hi])) else {
            break; // the -1 that ends the stream
        };
        packets.push(tail[..len].to_vec());
        rest = &tail[len..];
    }
    packets
});

/// A SILK voice of `ms` milliseconds the decoder reads (#574): the header WeChat writes, then a packet of the tone
/// per 20 ms, over again past its second. The random numbers it once drew are still drawn, so the rest of the
/// account stays the same.
pub(super) fn silk(rng: &mut Rng, ms: i64) -> Vec<u8> {
    let mut out = b"\x02#!SILK_V3".to_vec();
    for packet in TONE_PACKETS.iter().cycle().take((ms / 20).max(1) as usize) {
        let n = 20 + rng.below(26);
        let _ = rng.bytes(n);
        out.extend((packet.len() as u16).to_le_bytes());
        out.extend(packet);
    }
    out
}

/// The month folder of a file sent at `t` (`YYYY-MM` in the account's clock).
pub(super) fn month(t: i64) -> String {
    // days since 1970-01-01 to a civil date (Howard Hinnant's algorithm)
    let z = (t + TZ).div_euclid(DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y}-{m:02}")
}
