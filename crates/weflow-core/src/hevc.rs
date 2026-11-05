//! WXGF pictures: the first picture of a WXGF HEVC stream is decoded in Rust (`heic-rs`, an HEVC still-picture
//! decoder: WXGF images are single intra pictures, almost all of them Main Still Picture) and written as a JPEG the
//! way ffmpeg's `mjpeg` does it (the YCbCr planes kept, limited range scaled to the full range JPEG uses).
//!
//! On a real account 1763 WXGF images decoded to the same samples as ffmpeg, and all 5,668 of a whole account
//! decoded. What it does not cover (10-bit, 4:2:2 / 4:4:4, inter-predicted first pictures) or damaged data is an
//! [`Unsupported`], and the image is not converted.

/// JPEG quality: as close to the source as ffmpeg's `-q:v 2` (measured on real WXGF images; the files are about
/// a quarter larger).
const JPEG_QUALITY: u8 = 92;

/// Most luma samples a coded picture may have here: `MaxLumaPs` of level 6.2, the highest HEVC level, so no conforming
/// stream has a larger picture (the largest of the 5,668 WXGF pictures on a real account had 12.4 million). The
/// decoder allocates the whole coded picture before it reads a slice, and an SPS can declare up to 65535 × 65535
/// samples, so without this a damaged stream could ask for tens of gigabytes.
pub const MAX_LUMA_SAMPLES: u64 = 35_651_584;

/// Why the first picture of an HEVC stream is not turned into a JPEG here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unsupported {
    /// The stream has no slice of a first picture.
    NoPicture,
    /// A parameter set the picture needs is missing.
    ParameterSets,
    /// A coded picture larger than [`MAX_LUMA_SAMPLES`].
    TooLarge,
    /// Samples deeper than 8 bits (the luma bit depth).
    BitDepth(u8),
    /// 4:2:2 chroma.
    Chroma422,
    /// 4:4:4 chroma.
    Chroma444,
    /// The picture uses a coding tool the decoder does not implement, which it names (for example inter prediction
    /// in a first picture).
    Tool(&'static str),
    /// The stream breaks the HEVC syntax or ends early: damaged data.
    Damaged,
    /// The decoded picture could not be written as a JPEG.
    Jpeg,
}

impl Unsupported {
    /// Whether the stream is damaged, rather than a picture in a format not handled here yet.
    pub fn is_damaged(self) -> bool {
        matches!(
            self,
            Self::NoPicture | Self::ParameterSets | Self::TooLarge | Self::Damaged
        )
    }
}

/// The detail of the reason, in the terms of the format (an error message says whether it is damage).
impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPicture => f.write_str("no picture"),
            Self::ParameterSets => f.write_str("a parameter set is missing"),
            Self::TooLarge => f.write_str("larger than any HEVC level allows"),
            Self::BitDepth(bits) => write!(f, "{bits}-bit samples"),
            Self::Chroma422 => f.write_str("4:2:2 chroma"),
            Self::Chroma444 => f.write_str("4:4:4 chroma"),
            Self::Tool(tool) => f.write_str(tool),
            Self::Damaged => f.write_str("broken HEVC bitstream"),
            Self::Jpeg => f.write_str("JPEG encoding"),
        }
    }
}

impl From<heic_rs::Error> for Unsupported {
    fn from(e: heic_rs::Error) -> Self {
        match e {
            heic_rs::Error::MissingParameterSet(_) => Self::ParameterSets,
            heic_rs::Error::Unsupported(tool) => Self::Tool(tool),
            // the codec reports a broken or short bitstream as `Truncated` or `Malformed`; the other kinds belong to
            // the HEIF container
            _ => Self::Damaged,
        }
    }
}

/// The first picture of an Annex-B HEVC stream as a JPEG, or `None` when it cannot be decoded here.
pub fn to_jpeg(hevc: &[u8]) -> Option<Vec<u8>> {
    decode_to_jpeg(hevc).ok()
}

/// The first picture of an Annex-B HEVC stream as a JPEG, or why it cannot be decoded here.
///
/// # Errors
///
/// [`Unsupported`] says why: no picture, a picture this decoder or the JPEG writer does not handle, or damaged data.
pub fn decode_to_jpeg(hevc: &[u8]) -> Result<Vec<u8>, Unsupported> {
    let (sets, slices) = first_picture(hevc);
    if slices.is_empty() {
        return Err(Unsupported::NoPicture);
    }
    if largest_coded_picture(&sets).is_some_and(|samples| samples > MAX_LUMA_SAMPLES) {
        return Err(Unsupported::TooLarge);
    }
    let full_range = sets
        .iter()
        .rev()
        .find(|u| nal_type(u) == 33)
        .and_then(|sps| sps_full_range(sps))
        .unwrap_or(false);
    let frame = heic_rs::hevc::decode_still(&sets, &slices)?;
    encode(&frame, full_range)
}

/// Luma samples of the largest coded picture an SPS of the first picture of `hevc` declares, or `None` when it has
/// no SPS that can be read.
pub fn coded_luma_samples(hevc: &[u8]) -> Option<u64> {
    largest_coded_picture(&first_picture(hevc).0)
}

/// Luma samples of the largest coded picture the SPSs among `sets` declare (the decoder picks one by its id; an SPS
/// that cannot be read is left to the decoder).
fn largest_coded_picture(sets: &[&[u8]]) -> Option<u64> {
    sets.iter()
        .filter(|u| nal_type(u) == 33)
        .filter_map(|sps| {
            let size = SpsStart::read(&mut Bits::new(sps))?;
            Some(u64::from(size.width) * u64::from(size.height))
        })
        .max()
}

fn nal_type(unit: &[u8]) -> u8 {
    (unit[0] >> 1) & 0x3f
}

/// NAL units of an Annex-B stream, without start codes (and without the zero bytes trailing a unit).
fn split_annexb(buf: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (k, &start) in starts.iter().enumerate() {
        let mut end = starts.get(k + 1).map_or(buf.len(), |next| next - 3);
        while end > start && buf[end - 1] == 0 {
            end -= 1;
        }
        // a NAL unit header is two bytes, and the forbidden bit is zero
        if end >= start + 3 && buf[start] & 0x80 == 0 {
            units.push(&buf[start..end]);
        }
    }
    units
}

/// (parameter sets, slices) of the first picture of base layer 0.
fn first_picture(hevc: &[u8]) -> (Vec<&[u8]>, Vec<&[u8]>) {
    let (mut sets, mut slices) = (Vec::new(), Vec::new());
    for unit in split_annexb(hevc) {
        let layer = ((unit[0] & 1) << 5) | (unit[1] >> 3);
        if layer != 0 {
            continue;
        }
        match nal_type(unit) {
            0..=31 => {
                // first_slice_segment_in_pic_flag: the next picture starts
                if !slices.is_empty() && unit[2] & 0x80 != 0 {
                    break;
                }
                slices.push(unit);
            }
            32..=34 if slices.is_empty() => sets.push(unit),
            _ => {}
        }
    }
    (sets, slices)
}

/// A decoded 8-bit picture handed to the JPEG encoder row by row, straight from its planes (an interleaved copy of the
/// whole picture would cost three bytes a pixel on every media thread).
struct Planes<'a> {
    frame: &'a heic_rs::hevc::Frame,
    mono: bool,
    full_range: bool,
    width: u16,
    height: u16,
}

impl Planes<'_> {
    fn luma(&self, v: u16) -> u8 {
        if self.full_range {
            v.min(255) as u8
        } else {
            ((v as i32 - 16) * 255 + 109).div_euclid(219).clamp(0, 255) as u8
        }
    }

    fn chroma(&self, v: u16) -> u8 {
        if self.full_range {
            v.min(255) as u8
        } else {
            (((v as i32 - 128) * 255 + 112).div_euclid(224) + 128).clamp(0, 255) as u8
        }
    }
}

impl jpeg_encoder::ImageBuffer for Planes<'_> {
    fn get_jpeg_color_type(&self) -> jpeg_encoder::JpegColorType {
        if self.mono {
            jpeg_encoder::JpegColorType::Luma
        } else {
            jpeg_encoder::JpegColorType::Ycbcr
        }
    }

    fn width(&self) -> u16 {
        self.width
    }

    fn height(&self) -> u16 {
        self.height
    }

    fn fill_buffers(&self, y: u16, buffers: &mut [Vec<u8>; 4]) {
        let f = self.frame;
        let (r, w) = (y as usize, self.width as usize);
        let luma = &f.y[r * f.y_stride as usize..][..w];
        buffers[0].extend(luma.iter().map(|v| self.luma(*v)));
        if self.mono {
            return;
        }
        let at = (r / 2) * f.c_stride as usize;
        let (cb, cr) = (&f.cb[at..], &f.cr[at..]);
        for c in 0..w {
            buffers[1].push(self.chroma(cb[c / 2]));
            buffers[2].push(self.chroma(cr[c / 2]));
        }
    }
}

fn encode(frame: &heic_rs::hevc::Frame, full_range: bool) -> Result<Vec<u8>, Unsupported> {
    use heic_rs::hevc::ChromaFormat;
    if frame.bit_depth != 8 {
        return Err(Unsupported::BitDepth(frame.bit_depth));
    }
    if frame.validate().is_err() {
        return Err(Unsupported::Damaged);
    }
    let mono = match frame.chroma {
        ChromaFormat::Monochrome => true,
        ChromaFormat::Yuv420 => false,
        ChromaFormat::Yuv422 => return Err(Unsupported::Chroma422),
        ChromaFormat::Yuv444 => return Err(Unsupported::Chroma444),
    };
    let side = |v: u32| u16::try_from(v).map_err(|_| Unsupported::Jpeg);
    let planes = Planes {
        frame,
        mono,
        full_range,
        width: side(frame.width)?,
        height: side(frame.height)?,
    };
    let mut out = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, JPEG_QUALITY);
    if !mono {
        encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
    }
    encoder
        .encode_image(planes)
        .map_err(|_| Unsupported::Jpeg)?;
    Ok(out)
}

/// Reads an RBSP (emulation prevention bytes removed) bit by bit.
struct Bits {
    data: Vec<u8>,
    pos: usize,
}

impl Bits {
    fn new(unit: &[u8]) -> Self {
        let mut data = Vec::with_capacity(unit.len());
        let mut zeros = 0;
        for &b in unit {
            if zeros >= 2 && b == 3 {
                zeros = 0;
                continue;
            }
            zeros = if b == 0 { zeros + 1 } else { 0 };
            data.push(b);
        }
        Self { data, pos: 0 }
    }

    fn u(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8)?;
            v = (v << 1) | u32::from((byte >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Some(v)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        self.pos += n;
        (self.pos <= self.data.len() * 8).then_some(())
    }

    fn flag(&mut self) -> Option<bool> {
        self.u(1).map(|v| v == 1)
    }

    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while !self.flag()? {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1u32 << zeros) - 1 + self.u(zeros)?)
    }

    fn se(&mut self) -> Option<i64> {
        let k = i64::from(self.ue()?);
        Some(if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) })
    }
}

/// `video_full_range_flag` of a sequence parameter set (H.265 7.3.2.2), `None` when it carries no VUI signal type or
/// cannot be parsed. The decoder reads the SPS itself; this only walks to the VUI for the sample range.
pub fn sps_full_range(sps: &[u8]) -> Option<bool> {
    let mut b = Bits::new(sps);
    let SpsStart {
        max_sub_layers_minus1,
        ..
    } = SpsStart::read(&mut b)?;
    if b.flag()? {
        for _ in 0..4 {
            b.ue()?; // conformance window
        }
    }
    b.ue()?;
    b.ue()?; // bit depths
             // log2_max_pic_order_cnt_lsb_minus4 is at most 12; a damaged SPS may hold anything
    let log2_max_poc_lsb = b.ue()?.checked_add(4)?;
    let first = if b.flag()? { 0 } else { max_sub_layers_minus1 };
    for _ in first..=max_sub_layers_minus1 {
        for _ in 0..3 {
            b.ue()?; // max_dec_pic_buffering, max_num_reorder, max_latency_increase
        }
    }
    for _ in 0..6 {
        b.ue()?; // coding / transform block sizes and depths
    }
    if b.flag()? && b.flag()? {
        scaling_list_data(&mut b)?;
    }
    b.skip(2)?; // amp, sample_adaptive_offset
    if b.flag()? {
        b.skip(8)?; // pcm sample bit depths
        b.ue()?;
        b.ue()?;
        b.skip(1)?;
    }
    let sets = b.ue()?;
    if sets > 64 {
        return None;
    }
    let mut delta_pocs: Vec<u32> = Vec::with_capacity(sets as usize);
    for idx in 0..sets as usize {
        let n = st_ref_pic_set(&mut b, idx, &delta_pocs)?;
        delta_pocs.push(n);
    }
    if b.flag()? {
        for _ in 0..b.ue()? {
            b.skip(log2_max_poc_lsb as usize + 1)?; // lt_ref_pic_poc_lsb_sps, used_by_curr_pic_lt_sps_flag
        }
    }
    b.skip(2)?; // temporal mvp, strong intra smoothing
    if !b.flag()? {
        return None; // no VUI
    }
    if b.flag()? && b.u(8)? == 255 {
        b.skip(32)?; // aspect ratio: sar width and height
    }
    if b.flag()? {
        b.skip(1)?; // overscan_appropriate_flag
    }
    if !b.flag()? {
        return None; // no video signal type
    }
    b.skip(3)?; // video_format
    b.flag()
}

/// An SPS (H.265 7.3.2.2) up to the coded picture size.
struct SpsStart {
    max_sub_layers_minus1: u32,
    /// `pic_width_in_luma_samples`.
    width: u32,
    /// `pic_height_in_luma_samples`.
    height: u32,
}

impl SpsStart {
    /// Reads an SPS from its NAL unit header to the picture size, leaving `b` right after it.
    fn read(b: &mut Bits) -> Option<Self> {
        b.skip(16)?; // NAL unit header
        b.skip(4)?; // sps_video_parameter_set_id
        let max_sub_layers_minus1 = b.u(3)?;
        b.skip(1)?;
        profile_tier_level(b, max_sub_layers_minus1)?;
        b.ue()?; // sps_seq_parameter_set_id
        if b.ue()? == 3 {
            b.skip(1)?; // separate_colour_plane_flag
        }
        let width = b.ue()?;
        let height = b.ue()?;
        Some(Self {
            max_sub_layers_minus1,
            width,
            height,
        })
    }
}

fn profile_tier_level(b: &mut Bits, max_sub_layers_minus1: u32) -> Option<()> {
    b.skip(88 + 8)?; // general profile (88 bits) and general_level_idc
    let mut present = Vec::new();
    for _ in 0..max_sub_layers_minus1 {
        present.push((b.flag()?, b.flag()?));
    }
    if max_sub_layers_minus1 > 0 {
        b.skip(2 * (8 - max_sub_layers_minus1 as usize))?;
    }
    for (profile, level) in present {
        if profile {
            b.skip(88)?;
        }
        if level {
            b.skip(8)?;
        }
    }
    Some(())
}

fn scaling_list_data(b: &mut Bits) -> Option<()> {
    for size_id in 0..4u32 {
        let step = if size_id == 3 { 3 } else { 1 };
        for _ in (0..6).step_by(step) {
            if !b.flag()? {
                b.ue()?; // scaling_list_pred_matrix_id_delta
                continue;
            }
            if size_id > 1 {
                b.se()?; // dc coefficient
            }
            for _ in 0..64.min(1 << (4 + (size_id << 1))) {
                b.se()?;
            }
        }
    }
    Some(())
}

/// Parses `st_ref_pic_set(idx)` of an SPS and returns its NumDeltaPocs.
fn st_ref_pic_set(b: &mut Bits, idx: usize, delta_pocs: &[u32]) -> Option<u32> {
    if idx != 0 && b.flag()? {
        // inter_ref_pic_set_prediction_flag: predicted from the previous set (an SPS has no delta_idx)
        b.skip(1)?; // delta_rps_sign
        b.ue()?; // abs_delta_rps_minus1
        let mut n = 0;
        for _ in 0..=*delta_pocs.get(idx - 1)? {
            let used = b.flag()?;
            if used || b.flag()? {
                n += 1;
            }
        }
        return Some(n);
    }
    let (negative, positive) = (b.ue()?, b.ue()?);
    if negative > 16 || positive > 16 {
        return None;
    }
    for _ in 0..negative + positive {
        b.ue()?;
        b.skip(1)?;
    }
    Some(negative + positive)
}

#[cfg(test)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "test values are known to fit, or are meant to wrap"
)]
mod tests {
    use super::*;

    const FULL: &[u8] = include_bytes!("../testdata/wxgf/full_range.hevc");
    const LIMITED: &[u8] = include_bytes!("../testdata/wxgf/limited_range.hevc");

    fn sps(hevc: &[u8]) -> &[u8] {
        split_annexb(hevc)
            .into_iter()
            .find(|u| nal_type(u) == 33)
            .unwrap()
    }

    #[test]
    fn the_sample_range_is_read_from_the_vui() {
        assert_eq!(sps_full_range(sps(FULL)), Some(true));
        assert_eq!(sps_full_range(sps(LIMITED)), Some(false));
    }

    #[test]
    fn the_decoded_picture_is_ffmpegs() {
        // testdata/wxgf/*.yuv: ffmpeg's decode of the same streams (planar 4:2:0, 70x46)
        for (hevc, yuv) in [
            (FULL, &include_bytes!("../testdata/wxgf/full_range.yuv")[..]),
            (
                LIMITED,
                &include_bytes!("../testdata/wxgf/limited_range.yuv")[..],
            ),
        ] {
            let (sets, slices) = first_picture(hevc);
            let f = heic_rs::hevc::decode_still(&sets, &slices).unwrap();
            assert_eq!((f.width, f.height), (70, 46));
            let mut planar = Vec::new();
            for r in 0..46 {
                planar.extend(
                    f.y[r * f.y_stride as usize..][..70]
                        .iter()
                        .map(|v| *v as u8),
                );
            }
            for plane in [&f.cb, &f.cr] {
                for r in 0..23 {
                    planar.extend(
                        plane[r * f.c_stride as usize..][..35]
                            .iter()
                            .map(|v| *v as u8),
                    );
                }
            }
            assert_eq!(planar, yuv);
        }
    }

    #[test]
    fn a_jpeg_of_the_first_picture_in_the_full_range() {
        for (hevc, yuv, full) in [
            (
                FULL,
                &include_bytes!("../testdata/wxgf/full_range.yuv")[..],
                true,
            ),
            (
                LIMITED,
                &include_bytes!("../testdata/wxgf/limited_range.yuv")[..],
                false,
            ),
        ] {
            let jpg = to_jpeg(hevc).unwrap();
            assert_eq!(&jpg[..3], &[0xff, 0xd8, 0xff]);
            let mut d = jpeg_decoder::Decoder::new(&jpg[..]);
            let rgb = d.decode().unwrap();
            let info = d.info().unwrap();
            assert_eq!((info.width, info.height), (70, 46));
            // the luma the JPEG shows is the source's, scaled to the full range when it was limited
            let mut err = 0f64;
            for i in 0..70 * 46 {
                let (r, g, b) = (
                    rgb[i * 3] as f64,
                    rgb[i * 3 + 1] as f64,
                    rgb[i * 3 + 2] as f64,
                );
                let shown = 0.299 * r + 0.587 * g + 0.114 * b;
                let src = yuv[i] as f64;
                let want = if full {
                    src
                } else {
                    (src - 16.0) * 255.0 / 219.0
                };
                err += (shown - want).abs();
            }
            assert!(
                err / (70.0 * 46.0) < 4.0,
                "mean luma error {}",
                err / 3220.0
            );
        }
    }

    /// `sample` with its SPS declaring a `width` × `height` coded picture (the rest of the SPS and stream unchanged).
    fn with_picture_size(sample: &[u8], width: u32, height: u32) -> Vec<u8> {
        let ue_len = |v: u32| 2 * (u32::BITS - (v + 1).leading_zeros() - 1) as usize + 1;
        let mut stream = Vec::new();
        for unit in split_annexb(sample) {
            let unit = if nal_type(unit) == 33 {
                let mut b = Bits::new(unit);
                let old = SpsStart::read(&mut b).unwrap();
                let rbsp = b.data;
                let bit = |i: usize| (rbsp[i / 8] >> (7 - i % 8)) & 1 == 1;
                let after = b.pos;
                let before = after - ue_len(old.width) - ue_len(old.height);
                let stop = (0..rbsp.len() * 8).rev().find(|&i| bit(i)).unwrap();
                let mut bits: Vec<bool> = (0..before).map(bit).collect();
                for v in [width, height] {
                    let n = ue_len(v) / 2;
                    bits.extend(std::iter::repeat_n(false, n));
                    bits.extend((0..=n).rev().map(|k| (v + 1) >> k & 1 == 1));
                }
                bits.extend((after..stop).map(bit));
                bits.push(true);
                while !bits.len().is_multiple_of(8) {
                    bits.push(false);
                }
                // back to a NAL unit: emulation prevention bytes where the payload would look like a start code
                let mut out = Vec::new();
                let mut zeros = 0;
                for byte in bits
                    .chunks(8)
                    .map(|c| c.iter().fold(0u8, |a, &b| a << 1 | u8::from(b)))
                {
                    if zeros >= 2 && byte <= 3 {
                        out.push(3);
                        zeros = 0;
                    }
                    zeros = if byte == 0 { zeros + 1 } else { 0 };
                    out.push(byte);
                }
                out
            } else {
                unit.to_vec()
            };
            stream.extend_from_slice(&[0, 0, 0, 1]);
            stream.extend_from_slice(&unit);
        }
        stream
    }

    #[test]
    fn a_coded_picture_too_large_for_any_hevc_level_is_refused_before_decoding() {
        let sps = SpsStart::read(&mut Bits::new(sps(FULL))).unwrap();
        // the rewriting itself changes nothing when the size stays
        let same = with_picture_size(FULL, sps.width, sps.height);
        assert_eq!(split_annexb(&same), split_annexb(FULL));
        assert!(decode_to_jpeg(&same).is_ok());
        for (w, h) in [(65535, 65535), (8192, 8192), (1080, 40000)] {
            let big = with_picture_size(FULL, w, h);
            assert_eq!(coded_luma_samples(&big), Some(u64::from(w) * u64::from(h)));
            assert_eq!(decode_to_jpeg(&big), Err(Unsupported::TooLarge), "{w}x{h}");
        }
        // within the limit the decoder gets the stream, and finds the slices do not match the size
        let fits = with_picture_size(FULL, 4096, 4096);
        assert_ne!(decode_to_jpeg(&fits), Err(Unsupported::TooLarge));
    }

    /// A WXGF picture is a file on the user's disk, so it may be truncated or damaged: every prefix of a sample, and
    /// samples with random bytes changed, removed, repeated or inserted, give an error or a picture, never a panic
    /// (tests build with overflow checks, so an overflow panics here too).
    #[test]
    fn damaged_streams_give_an_error_and_never_panic() {
        use rand::{rngs::StdRng, RngExt, SeedableRng};
        for sample in [FULL, LIMITED] {
            for len in 0..=sample.len() {
                let _ = decode_to_jpeg(&sample[..len]);
            }
        }
        let mut rng = StdRng::seed_from_u64(494);
        let mut decoded = [0usize; 2];
        let mut reasons = std::collections::HashSet::new();
        for i in 0..3000 {
            let mut data = [FULL, LIMITED][i % 2].to_vec();
            for _ in 0..rng.random_range(1..=8) {
                if data.is_empty() {
                    break;
                }
                let at = rng.random_range(0..data.len());
                let end = (at + rng.random_range(1..32)).min(data.len());
                match rng.random_range(0..4) {
                    0 => data[at] ^= 1 << rng.random_range(0..8),
                    1 => data[at] = rng.random(),
                    2 => {
                        data.drain(at..end);
                    }
                    _ => {
                        let piece = data[at..end].to_vec();
                        data.splice(at..at, piece);
                    }
                }
            }
            for unit in split_annexb(&data) {
                let _ = sps_full_range(unit);
            }
            let result = decode_to_jpeg(&data);
            decoded[usize::from(result.is_ok())] += 1;
            if let Err(e) = result {
                reasons.insert(std::mem::discriminant(&e));
            }
            // and inside a WXGF container, through the candidate streams the image service tries
            let mut wxgf = b"wxgf".to_vec();
            wxgf.extend_from_slice(&[0; 16]);
            wxgf.extend_from_slice(&data);
            for (_, stream) in crate::image::wxgf_hevc_candidates(&wxgf) {
                let _ = decode_to_jpeg(&stream);
            }
        }
        // the damage reaches the decoder: some streams still decode, and the others fail for several reasons
        assert!(decoded.iter().all(|&n| n > 0), "{decoded:?}");
        assert!(reasons.len() >= 3, "{reasons:?}");
    }

    #[test]
    fn a_stream_without_a_picture_is_not_decoded() {
        assert!(to_jpeg(&[]).is_none());
        assert!(to_jpeg(&FULL[..FULL.len() / 3]).is_none());
        let sets_only: Vec<u8> = split_annexb(FULL)
            .into_iter()
            .filter(|u| nal_type(u) >= 32)
            .flat_map(|u| [&[0, 0, 0, 1][..], u].concat())
            .collect();
        assert!(to_jpeg(&sets_only).is_none());
    }
}
