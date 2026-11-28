//! 基准账号的文件：图片、视频、语音，以及加密后的 `.dat` 文件。

use aes::cipher::{BlockCipherEncrypt, KeyInit};

use super::rng::Rng;
use super::{DAY, IMAGE_AES_KEY, IMAGE_XOR_KEY, TZ};

/// V2 格式的 `.dat` 文件：开头用 [`IMAGE_AES_KEY`] 做 AES 加密，结尾用 [`IMAGE_XOR_KEY`] 做 XOR。
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

/// `len` 字节的 JPEG：JFIF 文件头、填满随机字节的注释段和结束标记（不是真的图片）。
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

/// 只有一个灰色像素的合法 PNG，用一个填满随机字节的私有块补足 `len` 字节。
pub(super) fn png(rng: &mut Rng, len: usize) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png_chunk(&mut out, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]);
    // zlib 数据：扫描线（过滤器 0，灰度 0x80）的一个不压缩块，以及它的 Adler-32
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

/// WXGF 图片的 HEVC 帧，最小的在前：完整的图片，尺寸与真实账号中同样文件大小的 WXGF 图片相同（见
/// `testdata/bench_wxgf`，#1221），转换它们的开销才与真实账号一样。
const WXGF_FRAMES: [&[u8]; 6] = [
    include_bytes!("../../../testdata/bench_wxgf/wxgf_800x480.hevc"),
    include_bytes!("../../../testdata/bench_wxgf/wxgf_672x1200.hevc"),
    include_bytes!("../../../testdata/bench_wxgf/wxgf_1600x900.hevc"),
    include_bytes!("../../../testdata/bench_wxgf/wxgf_1920x1080.hevc"),
    include_bytes!("../../../testdata/bench_wxgf/wxgf_2048x1152.hevc"),
    include_bytes!("../../../testdata/bench_wxgf/wxgf_1152x2048.hevc"),
];

/// 内置解码器能读的 WXGF 图片：取 [`WXGF_FRAMES`] 中放得进 `len` 字节的最大一帧（都放不下时取最小的），
/// 再用填充 NAL 单元补足 `len` 字节。
pub(super) fn wxgf(len: usize) -> Vec<u8> {
    const HEADER: [u8; 10] = *b"wxgf\x24\x01\x00\x46\x00\x2e";
    let frame = WXGF_FRAMES
        .iter()
        .rev()
        .find(|f| HEADER.len() + f.len() <= len)
        .unwrap_or(&WXGF_FRAMES[0]);
    let mut out = HEADER.to_vec();
    out.extend_from_slice(frame);
    while out.len() + 6 < len {
        // 填充数据（NAL 类型 38）：0xFF 字节，直到结束位
        let n = (len - out.len() - 6).min(60_000);
        out.extend([0x00, 0x00, 0x01, 0x4C, 0x01]);
        out.extend(std::iter::repeat_n(0xFF, n));
        out.push(0x80);
    }
    out
}

/// `len` 字节的 MP4：一个文件类型盒（`ftyp`）和一个填满随机字节的媒体数据盒（`mdat`）。
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

/// SILK SDK 以 24 kHz 编码的一秒 440 Hz 音调的数据包（每包 20 毫秒）。
static TONE_PACKETS: std::sync::LazyLock<Vec<Vec<u8>>> = std::sync::LazyLock::new(|| {
    let stream = weflow_silk::testenc::encode_tone(24_000);
    let mut packets = Vec::new();
    let mut rest = &stream[b"#!SILK_V3".len()..];
    while let [lo, hi, tail @ ..] = rest {
        let Ok(len) = usize::try_from(i16::from_le_bytes([*lo, *hi])) else {
            break; // 结束数据流的 -1
        };
        packets.push(tail[..len].to_vec());
        rest = &tail[len..];
    }
    packets
});

/// 解码器能读的 `ms` 毫秒的 SILK 语音（#574）：先是微信写的文件头，
/// 然后每 20 毫秒一个音调的数据包，超过一秒就从头循环。每个包还从生成器取一段随机长度的随机字节（不使用），
/// 这样账号其余部分从生成器取到的随机数与音调无关。
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

/// 在 `t` 时发送的文件所在的月份目录（按账号的时区，`YYYY-MM`）。
pub(super) fn month(t: i64) -> String {
    let (y, m, _) = civil(t);
    format!("{y}-{m:02}")
}

/// 按账号的时区，`t` 的年、月、日。
pub(super) fn civil(t: i64) -> (i64, i64, i64) {
    // 从 1970-01-01 起的天数换算成公历日期（Howard Hinnant 的算法）
    let z = (t + TZ).div_euclid(DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
