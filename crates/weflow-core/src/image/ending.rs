//! Where a picture's own bytes end, read from its structure: whether a decrypted `.dat` holds the whole picture or
//! stops inside it, as a file WeChat is still writing does (#1112).
//!
//! Only the structure is walked: JPEG markers and segment lengths, PNG chunks, GIF blocks, the RIFF size of a WebP.
//! Nothing is decoded and no checksum is checked. Bytes after the end (a motion photo's video, padding) are not part of
//! the picture and do not make it incomplete.

use super::ImageFormat;

/// Whether bytes hold a whole picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// The picture ends at this offset.
    At(usize),
    /// The bytes stop inside the picture, or break its structure before its end.
    CutShort,
    /// Not one of the [`ImageFormat`]s (a WXGF container, say), whose end is not read here.
    Unknown,
}

/// Where the picture in `data` ends.
pub fn of(data: &[u8]) -> Ending {
    let end = match ImageFormat::of(data) {
        Some(ImageFormat::Jpeg) => jpeg_end(data),
        Some(ImageFormat::Png) => png_end(data),
        Some(ImageFormat::Gif) => gif_end(data),
        Some(ImageFormat::Webp) => webp_end(data),
        None => return Ending::Unknown,
    };
    end.map_or(Ending::CutShort, Ending::At)
}

/// Whether `data` stops inside its picture: [`of`] is [`Ending::CutShort`]. A JPEG ending in its EOI marker or a PNG
/// in its IEND chunk is taken as whole without walking it (a cut that ends there by chance is one in tens of
/// thousands).
pub fn is_cut_short(data: &[u8]) -> bool {
    let closed = match ImageFormat::of(data) {
        Some(ImageFormat::Jpeg) => data.ends_with(&[0xFF, 0xD9]),
        Some(ImageFormat::Png) => data.ends_with(b"IEND\xAE\x42\x60\x82"),
        Some(ImageFormat::Gif | ImageFormat::Webp) | None => false,
    };
    !closed && of(data) == Ending::CutShort
}

/// The end of a JPEG's EOI marker. Segments are skipped by their length; entropy-coded data runs to the next marker
/// that is not a stuffed zero or a restart. Bytes where a marker should start are skipped up to the next `FF`, as
/// libjpeg does ("extraneous bytes before marker").
///
/// After the first scan only the markers that may come between scans or end the picture are taken: in noise (the
/// tail of a `.dat` decrypted before WeChat finished it) any other one breaks the structure, where skipping it by a
/// made-up length would go on until an `FF D9` turns up by chance.
fn jpeg_end(d: &[u8]) -> Option<usize> {
    // past SOI
    let mut i = 2;
    let mut scanned = false;
    loop {
        i += memchr::memchr(0xFF, d.get(i..)?)?;
        // fill bytes before the marker's code
        while *d.get(i)? == 0xFF {
            i += 1;
        }
        let code = d[i];
        i += 1;
        match code {
            0xD9 => return Some(i),
            // a stuffed zero outside entropy-coded data, TEM or a restart: no length follows
            0x00 | 0x01 | 0xD0..=0xD7 if !scanned => continue,
            // what may come between scans: DHT, SOS, DQT, DNL, DRI, APPn, COM
            0xC4 | 0xDA..=0xDD | 0xE0..=0xEF | 0xFE => {}
            _ if scanned => return None,
            _ => {}
        }
        let len = usize::from(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]));
        i = i
            .checked_add(len)
            .filter(|&next| len >= 2 && next <= d.len())?;
        if code == 0xDA {
            scanned = true;
            i = scan_end(d, i)?;
        }
    }
}

/// The `FF` of the marker that ends the entropy-coded data starting at `i`.
fn scan_end(d: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let ff = i + memchr::memchr(0xFF, d.get(i..)?)?;
        match *d.get(ff + 1)? {
            0x00 | 0xD0..=0xD7 => i = ff + 2,
            _ => return Some(ff),
        }
    }
}

/// The end of a PNG's IEND chunk: chunks are a length, a type, the data and a CRC.
fn png_end(d: &[u8]) -> Option<usize> {
    // past the signature
    let mut i = 8;
    loop {
        let len = u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?);
        let next = usize::try_from(len)
            .ok()
            .and_then(|len| i.checked_add(12)?.checked_add(len))
            .filter(|&next| next <= d.len())?;
        if d.get(i + 4..i + 8)? == b"IEND" {
            return Some(next);
        }
        i = next;
    }
}

/// The end of a GIF's trailer (`3B`): after the header and the global colour table come extensions and images, each
/// a run of sub-blocks closed by an empty one.
fn gif_end(d: &[u8]) -> Option<usize> {
    let mut i = 13 + colour_table_len(*d.get(10)?);
    loop {
        match *d.get(i)? {
            0x3B => return Some(i + 1),
            // introducer, label
            0x21 => i = sub_blocks_end(d, i + 2)?,
            // the 10-byte descriptor, the local colour table, the LZW minimum code size
            0x2C => i = sub_blocks_end(d, i + 10 + colour_table_len(*d.get(i + 9)?) + 1)?,
            _ => return None,
        }
    }
}

/// The bytes of the colour table a GIF's packed field describes.
fn colour_table_len(flags: u8) -> usize {
    if flags & 0x80 == 0 {
        0
    } else {
        3 << ((flags & 0x07) + 1)
    }
}

/// The end of the run of GIF sub-blocks starting at `i`.
fn sub_blocks_end(d: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let len = usize::from(*d.get(i)?);
        i += 1 + len;
        if len == 0 {
            return Some(i);
        }
    }
}

/// The end of a WebP's RIFF chunk, by the size its header gives.
fn webp_end(d: &[u8]) -> Option<usize> {
    let size = u32::from_le_bytes(d.get(4..8)?.try_into().ok()?);
    usize::try_from(size)
        .ok()
        .and_then(|size| size.checked_add(8))
        .filter(|&end| end <= d.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// A JPEG of `width` × `height` grey-ish pixels from `seed`, written by `jpeg-encoder`.
    fn jpeg(width: u16, height: u16, seed: u8, progressive: bool, restart: u16) -> Vec<u8> {
        let pixels: Vec<u8> = (0..usize::from(width) * usize::from(height) * 3)
            .map(|i| {
                let i = u8::try_from(i % 251).expect("below 251");
                i.wrapping_mul(seed).wrapping_add(seed)
            })
            .collect();
        let mut out = Vec::new();
        let mut encoder = jpeg_encoder::Encoder::new(&mut out, 85);
        encoder.set_progressive(progressive);
        encoder.set_restart_interval(restart);
        encoder
            .encode(&pixels, width, height, jpeg_encoder::ColorType::Rgb)
            .expect("an encodable picture");
        out
    }

    /// A PNG of one row of `width` pixels in `chunks` IDAT chunks (the data is not a valid deflate stream: only the
    /// chunks are walked).
    fn png(width: u32, chunks: usize) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |kind: &[u8], data: &[u8]| {
            out.extend_from_slice(&u32::try_from(data.len()).expect("small").to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            out.extend_from_slice(&[0; 4]);
        };
        let mut ihdr = width.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&[0, 0, 0, 1, 8, 2, 0, 0, 0]);
        chunk(b"IHDR", &ihdr);
        for n in 0..chunks {
            chunk(
                b"IDAT",
                &vec![u8::try_from(n % 256).expect("below 256"); 40 + n],
            );
        }
        chunk(b"IEND", &[]);
        out
    }

    /// A GIF with a global colour table, a comment extension and `frames` images.
    fn gif(frames: usize) -> Vec<u8> {
        let mut out = b"GIF89a".to_vec();
        // 4 × 4, a global table of 4 colours
        out.extend_from_slice(&[4, 0, 4, 0, 0x81, 0, 0]);
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(&[0x21, 0xFE, 3, b'a', b'b', b'c', 0]);
        for _ in 0..frames {
            out.extend_from_slice(&[0x2C, 0, 0, 0, 0, 4, 0, 4, 0, 0, 2, 3, 1, 2, 3, 2, 4, 5, 0]);
        }
        out.push(0x3B);
        out
    }

    fn webp(payload: usize) -> Vec<u8> {
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&u32::try_from(payload + 4).expect("small").to_le_bytes());
        out.extend_from_slice(b"WEBP");
        out.extend(std::iter::repeat_n(7, payload));
        out
    }

    #[test]
    fn a_whole_picture_ends_where_its_format_says() {
        for (name, picture) in [
            ("baseline", jpeg(33, 17, 3, false, 0)),
            ("progressive", jpeg(33, 17, 5, true, 0)),
            ("restarts", jpeg(64, 40, 7, false, 2)),
            ("png", png(5, 3)),
            ("gif", gif(2)),
            ("webp", webp(30)),
        ] {
            assert_eq!(of(&picture), Ending::At(picture.len()), "{name}");
        }
        assert_eq!(of(b"wxgf\x00\x01\x02\x03\x04\x05\x06\x07"), Ending::Unknown);
    }

    #[test]
    fn bytes_after_the_end_are_not_part_of_the_picture() {
        let picture = jpeg(16, 16, 9, false, 0);
        let mut with_video = picture.clone();
        with_video.extend_from_slice(b"\x00\x00\x00\x18ftypmp42 and the rest of a video");
        assert_eq!(of(&with_video), Ending::At(picture.len()));
        let mut padded = png(3, 1);
        let end = padded.len();
        padded.extend_from_slice(&[0; 100]);
        assert_eq!(of(&padded), Ending::At(end));
    }

    #[test]
    fn stray_bytes_between_jpeg_segments_are_skipped() {
        let picture = jpeg(16, 16, 11, false, 0);
        // after the APP0 segment, which `jpeg-encoder` writes first
        let app0_end = 4 + usize::from(u16::from_be_bytes([picture[4], picture[5]]));
        let mut stray = picture[..app0_end].to_vec();
        stray.extend_from_slice(&[0, 0, 0x20]);
        stray.extend_from_slice(&picture[app0_end..]);
        assert_eq!(of(&stray), Ending::At(stray.len()));
    }

    /// A `.dat` WeChat was still writing, decrypted: the last bytes the header calls the XOR-ed tail are taken from
    /// the middle and XOR-ed, so the picture's real tail is missing and noise stands in its place.
    fn with_noise_tail(picture: &[u8], cut: usize, tail: usize, key: u8) -> Vec<u8> {
        let mut out = picture[..cut].to_vec();
        let start = out.len().saturating_sub(tail);
        for b in &mut out[start..] {
            *b ^= key;
        }
        out
    }

    #[test]
    fn a_picture_with_noise_for_its_tail_is_cut_short() {
        let picture = jpeg(48, 48, 13, false, 0);
        let cut = picture.len() * 2 / 3;
        assert_eq!(
            of(&with_noise_tail(&picture, cut, 64, 0x37)),
            Ending::CutShort
        );
    }

    proptest! {
        #![proptest_config(crate::property_cases(64))]

        #[test]
        fn any_prefix_of_a_picture_is_cut_short(
            width in 1u16..40,
            height in 1u16..40,
            seed in any::<u8>(),
            progressive in any::<bool>(),
            restart in 0u16..4,
            frames in 0usize..4,
            chunks in 0usize..4,
            cut in any::<prop::sample::Index>(),
        ) {
            for picture in [
                jpeg(width, height, seed, progressive, restart),
                png(u32::from(width), chunks),
                gif(frames),
                webp(usize::from(width)),
            ] {
                prop_assert_eq!(of(&picture), Ending::At(picture.len()));
                // at least the bytes that tell the format apart
                let cut = 12 + cut.index(picture.len() - 12);
                prop_assert_eq!(of(&picture[..cut]), Ending::CutShort, "cut at {} of {}", cut, picture.len());
            }
        }
    }
}
