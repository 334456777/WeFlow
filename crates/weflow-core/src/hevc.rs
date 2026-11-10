//! WXGF pictures: the first picture of a WXGF HEVC stream is decoded in Rust (`heic-rs`, an HEVC still-picture
//! decoder: WXGF images are single intra pictures, almost all of them Main Still Picture) and written as a JPEG the
//! way ffmpeg's `mjpeg` does it: the YCbCr planes kept at the chroma resolution they have (4:2:0, 4:2:2, 4:4:4 or
//! grey), deeper samples (Main 10, Rext) rounded to 8 bits, limited range scaled to the full range JPEG uses.
//!
//! On a real account 1763 WXGF images decoded to the same samples as ffmpeg, and all 5,668 of a whole account
//! decoded. heic-rs is patched (see the workspace `Cargo.toml`) to decode 4:2:2 and 4:4:4 intra pictures; the
//! range-extension coding tools it does not implement are left to a second decoder, rust_h265. An inter-predicted
//! first picture, or damaged data, is an [`Unsupported`], and the image is not converted.

/// JPEG quality: as close to the source as ffmpeg's `-q:v 2` (measured on real WXGF images; the files are about
/// a quarter larger).
const JPEG_QUALITY: u8 = 92;

/// Most luma samples a coded picture may have here: `MaxLumaPs` of level 6.2, the highest HEVC level, so no conforming
/// stream has a larger picture (the largest of the 5,668 WXGF pictures on a real account had 12.4 million). The
/// decoder allocates the whole coded picture before it reads a slice, and an SPS can declare up to 65535 × 65535
/// samples, so without this a damaged stream could ask for tens of gigabytes.
const MAX_LUMA_SAMPLES: u64 = 35_651_584;

/// Why the first picture of an HEVC stream is not turned into a JPEG here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unsupported {
    /// The stream has no slice of a first picture.
    NoPicture,
    /// A parameter set the picture needs is missing.
    ParameterSets,
    /// A coded picture larger than [`MAX_LUMA_SAMPLES`].
    TooLarge,
    /// Samples of a bit depth a JPEG cannot be made from (outside 8 to 16 bits).
    BitDepth(u8),
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
    let sps = sets.iter().rev().find(|u| nal_type(u) == 33);
    let signal = Signal {
        full_range: sps.and_then(|sps| sps_full_range(sps)).unwrap_or(false),
        chroma_bit_depth: sps.and_then(|sps| sps_chroma_bit_depth(sps)),
    };
    let still = match heic_rs::hevc::decode_still(&sets, &slices) {
        Ok(frame) => return encode(&Picture::of_heic(&frame, signal.chroma_bit_depth)?, signal),
        Err(e) => Unsupported::from(e),
    };
    // A picture heic-rs does not decode (a range-extension coding tool it does not implement, such as RDPCM or
    // extended precision) gets a second try with rust_h265, which implements them all. Only an IRAP picture: a video decoder would predict any other from mid-grey
    // references it makes up, and give a wrong picture rather than an error.
    if !(16..=23).contains(&nal_type(slices[0])) {
        return Err(still);
    }
    match decode_with_rust_h265(&sets, &slices) {
        Ok(frame) => encode(
            &Picture::of_rust_h265(&frame, signal.chroma_bit_depth)?,
            signal,
        ),
        // what the second decoder says it does not support; otherwise the first one's reason
        Err(Unsupported::Tool(tool)) => Err(Unsupported::Tool(tool)),
        Err(_) => Err(still),
    }
}

/// The first picture with rust_h265, from its parameter sets and slices.
fn decode_with_rust_h265(
    sets: &[&[u8]],
    slices: &[&[u8]],
) -> Result<rust_h265::Frame, Unsupported> {
    // one thread, so that an error in the slice data comes back from the call that fed the slice
    let mut decoder = rust_h265::Decoder::new();
    for unit in sets.iter().chain(slices) {
        let nal = rust_h265::parse_nal(unit).ok_or(Unsupported::Damaged)?;
        decoder.decode_nal(&nal)?;
    }
    decoder.flush().ok_or(Unsupported::NoPicture)
}

impl From<rust_h265::DecodeError> for Unsupported {
    fn from(e: rust_h265::DecodeError) -> Self {
        match e {
            rust_h265::DecodeError::Unsupported(tool) => Self::Tool(tool),
            rust_h265::DecodeError::ResourceLimit(_) => Self::TooLarge,
            // a broken or short bitstream, and whatever later releases add
            _ => Self::Damaged,
        }
    }
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

/// What the SPS says about the samples that a decoded frame does not.
#[derive(Clone, Copy)]
struct Signal {
    full_range: bool,
    /// `BitDepthC`, when it was read (a frame tells only the luma bit depth).
    chroma_bit_depth: Option<u8>,
}

/// How the chroma planes of a picture are laid out against its luma plane.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    Mono,
    /// Chroma rows and columns are `1 << y` and `1 << x` luma ones: 4:2:0 is (1, 1), 4:2:2 (1, 0), 4:4:4 (0, 0).
    Subsampled {
        x: u32,
        y: u32,
    },
}

impl Layout {
    /// The chroma plane size of a `width` × `height` picture.
    fn chroma_size(self, width: usize, height: usize) -> (usize, usize) {
        match self {
            Self::Mono => (0, 0),
            Self::Subsampled { x, y } => (width.div_ceil(1 << x), height.div_ceil(1 << y)),
        }
    }
}

/// The samples of one plane, at the width each decoder gives them.
#[derive(Clone, Copy)]
enum Samples<'a> {
    U8(&'a [u8]),
    U16(&'a [u16]),
}

impl Samples<'_> {
    fn len(self) -> usize {
        match self {
            Self::U8(s) => s.len(),
            Self::U16(s) => s.len(),
        }
    }

    /// Appends `width` samples of the row starting at `start` to `out`, each column `c` taken from `c >> x` and
    /// brought to 8 bits by `to8`.
    fn push_row(
        self,
        start: usize,
        width: usize,
        x: u32,
        out: &mut Vec<u8>,
        to8: impl Fn(u16) -> u8,
    ) {
        fn push<T: Copy + Into<u16>>(
            row: &[T],
            width: usize,
            x: u32,
            out: &mut Vec<u8>,
            to8: impl Fn(u16) -> u8,
        ) {
            if x == 0 {
                out.extend(row[..width].iter().map(|&v| to8(v.into())));
            } else {
                out.extend((0..width).map(|c| to8(row[c >> x].into())));
            }
        }
        match self {
            Self::U8(s) => push(&s[start..], width, x, out, to8),
            Self::U16(s) => push(&s[start..], width, x, out, to8),
        }
    }
}

/// A decoded picture, whichever decoder gave it, its planes checked to be as large as its geometry needs.
struct Picture<'a> {
    width: u16,
    height: u16,
    layout: Layout,
    y: Samples<'a>,
    cb: Samples<'a>,
    cr: Samples<'a>,
    y_stride: usize,
    c_stride: usize,
    /// Bits more than 8 in a luma sample, and in a chroma sample.
    luma_shift: u32,
    chroma_shift: u32,
}

impl<'a> Picture<'a> {
    fn of_heic(
        f: &'a heic_rs::hevc::Frame,
        chroma_bit_depth: Option<u8>,
    ) -> Result<Self, Unsupported> {
        use heic_rs::hevc::ChromaFormat;
        let layout = match f.chroma {
            ChromaFormat::Monochrome => Layout::Mono,
            ChromaFormat::Yuv420 => Layout::Subsampled { x: 1, y: 1 },
            ChromaFormat::Yuv422 => Layout::Subsampled { x: 1, y: 0 },
            ChromaFormat::Yuv444 => Layout::Subsampled { x: 0, y: 0 },
        };
        Self::new(
            (f.width, f.height),
            layout,
            [Samples::U16(&f.y), Samples::U16(&f.cb), Samples::U16(&f.cr)],
            (f.y_stride as usize, f.c_stride as usize),
            (f.bit_depth, chroma_bit_depth),
        )
    }

    fn of_rust_h265(
        f: &'a rust_h265::Frame,
        chroma_bit_depth: Option<u8>,
    ) -> Result<Self, Unsupported> {
        let layout = match f.chroma_format_idc {
            0 => Layout::Mono,
            1 => Layout::Subsampled { x: 1, y: 1 },
            2 => Layout::Subsampled { x: 1, y: 0 },
            3 => Layout::Subsampled { x: 0, y: 0 },
            _ => return Err(Unsupported::Damaged),
        };
        let plane = |p: &'a rust_h265::PixelData| match p {
            rust_h265::PixelData::U8(s) => Samples::U8(s),
            rust_h265::PixelData::U16(s) => Samples::U16(s),
        };
        // the planes are cropped, a row for each line
        let (cw, _) = layout.chroma_size(f.width as usize, f.height as usize);
        Self::new(
            (f.width, f.height),
            layout,
            [plane(&f.y), plane(&f.u), plane(&f.v)],
            (f.width as usize, cw),
            (f.bit_depth, chroma_bit_depth),
        )
    }

    fn new(
        (width, height): (u32, u32),
        layout: Layout,
        [y, cb, cr]: [Samples<'a>; 3],
        (y_stride, c_stride): (usize, usize),
        (luma_bits, chroma_bits): (u8, Option<u8>),
    ) -> Result<Self, Unsupported> {
        let shift = |bits: u8| {
            (8..=16)
                .contains(&bits)
                .then(|| u32::from(bits - 8))
                .ok_or(Unsupported::BitDepth(bits))
        };
        let luma_shift = shift(luma_bits)?;
        let chroma_shift = shift(chroma_bits.unwrap_or(luma_bits))?;
        let side = |v: u32| u16::try_from(v).map_err(|_| Unsupported::Jpeg);
        let (w, h) = (side(width)?, side(height)?);
        let (cw, ch) = layout.chroma_size(w.into(), h.into());
        let fits = |plane: Samples<'_>, stride: usize, (w, h): (usize, usize)| {
            h == 0 || (stride >= w && plane.len() >= stride * (h - 1) + w)
        };
        if w == 0
            || h == 0
            || !fits(y, y_stride, (w.into(), h.into()))
            || !fits(cb, c_stride, (cw, ch))
            || !fits(cr, c_stride, (cw, ch))
        {
            return Err(Unsupported::Damaged);
        }
        Ok(Self {
            width: w,
            height: h,
            layout,
            y,
            cb,
            cr,
            y_stride,
            c_stride,
            luma_shift,
            chroma_shift,
        })
    }
}

/// A picture handed to the JPEG encoder row by row, straight from its planes (an interleaved copy of the whole
/// picture would cost three bytes a pixel on every media thread), its samples brought to the 8 bits a JPEG has.
struct Planes<'a> {
    picture: &'a Picture<'a>,
    full_range: bool,
}

/// A sample `shift` bits deeper than 8 as an 8-bit full-range one. Full range is rounded to 8 bits. Limited range,
/// `(offset, range, centre)` at 8 bits (luma 16 to 16 + 219 from 0, chroma 128 ± 112 around 128), is also scaled to
/// the full range, the way ffmpeg's `mjpeg` does it at 8 bits.
fn to_8_bits(v: u16, shift: u32, full_range: bool, (offset, range, centre): (i32, i32, i32)) -> u8 {
    let v = i32::from(v);
    if full_range {
        return ((v + ((1 << shift) >> 1)) >> shift).clamp(0, 255) as u8;
    }
    let range = range << shift;
    let scaled = ((v - (offset << shift)) * 255 + range / 2).div_euclid(range);
    (scaled + centre).clamp(0, 255) as u8
}

impl jpeg_encoder::ImageBuffer for Planes<'_> {
    fn get_jpeg_color_type(&self) -> jpeg_encoder::JpegColorType {
        if self.picture.layout == Layout::Mono {
            jpeg_encoder::JpegColorType::Luma
        } else {
            jpeg_encoder::JpegColorType::Ycbcr
        }
    }

    fn width(&self) -> u16 {
        self.picture.width
    }

    fn height(&self) -> u16 {
        self.picture.height
    }

    fn fill_buffers(&self, y: u16, buffers: &mut [Vec<u8>; 4]) {
        let p = self.picture;
        let (r, w) = (usize::from(y), usize::from(p.width));
        let full = self.full_range;
        let luma = |v| to_8_bits(v, p.luma_shift, full, (16, 219, 0));
        let chroma = |v| to_8_bits(v, p.chroma_shift, full, (128, 224, 128));
        p.y.push_row(r * p.y_stride, w, 0, &mut buffers[0], luma);
        let Layout::Subsampled { x, y } = p.layout else {
            return;
        };
        let at = (r >> y) * p.c_stride;
        let [_, cb_out, cr_out, _] = buffers;
        p.cb.push_row(at, w, x, cb_out, chroma);
        p.cr.push_row(at, w, x, cr_out, chroma);
    }
}

/// `picture` as a JPEG, its chroma kept at the resolution the picture has it.
fn encode(picture: &Picture<'_>, signal: Signal) -> Result<Vec<u8>, Unsupported> {
    let mut out = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, JPEG_QUALITY);
    match picture.layout {
        Layout::Mono => {}
        Layout::Subsampled { x: 1, y: 1 } => {
            encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2)
        }
        Layout::Subsampled { x: 1, .. } => {
            encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_1)
        }
        Layout::Subsampled { .. } => {
            encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1)
        }
    }
    let planes = Planes {
        picture,
        full_range: signal.full_range,
    };
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
fn sps_full_range(sps: &[u8]) -> Option<bool> {
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

/// `BitDepthC` of a sequence parameter set (H.265 7.3.2.2), which may differ from the luma one.
fn sps_chroma_bit_depth(sps: &[u8]) -> Option<u8> {
    let mut b = Bits::new(sps);
    SpsStart::read(&mut b)?;
    if b.flag()? {
        for _ in 0..4 {
            b.ue()?; // conformance window
        }
    }
    b.ue()?; // bit_depth_luma_minus8
    u8::try_from(b.ue()?).ok()?.checked_add(8)
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

    /// A stream of another layout or bit depth (testdata/wxgf/README.md).
    struct Sample {
        name: &'static str,
        hevc: &'static [u8],
        /// ffmpeg's decode of it.
        yuv: &'static [u8],
        bits: u32,
        /// The chroma subsampling as (x, y) shifts; `None` for grey.
        chroma: Option<(usize, usize)>,
        full: bool,
    }

    macro_rules! layout {
        ($name:literal, $bits:expr, $chroma:expr, $full:expr) => {
            Sample {
                name: $name,
                hevc: include_bytes!(concat!("../testdata/wxgf/", $name, ".hevc")),
                yuv: include_bytes!(concat!("../testdata/wxgf/", $name, ".yuv")),
                bits: $bits,
                chroma: $chroma,
                full: $full,
            }
        };
    }

    const SAMPLES: &[Sample] = &[
        layout!("full_range", 8, Some((1, 1)), true),
        layout!("limited_range", 8, Some((1, 1)), false),
        layout!("yuv420p10_full", 10, Some((1, 1)), true),
        layout!("yuv420p10_limited", 10, Some((1, 1)), false),
        layout!("yuv422p_full", 8, Some((1, 0)), true),
        layout!("yuv444p_limited", 8, Some((0, 0)), false),
        layout!("gray10_full", 10, None, true),
    ];

    /// The chroma plane size of a 70x46 picture subsampled by `(x, y)`.
    fn chroma_size((x, y): (usize, usize)) -> (usize, usize) {
        (70usize.div_ceil(1 << x), 46usize.div_ceil(1 << y))
    }

    /// ffmpeg's planes of `l`: luma, then Cb and Cr (empty for grey).
    fn ffmpeg_planes(l: &Sample) -> [Vec<u16>; 3] {
        let samples: Vec<u16> = if l.bits > 8 {
            l.yuv
                .chunks(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect()
        } else {
            l.yuv.iter().map(|&b| u16::from(b)).collect()
        };
        let luma = 70 * 46;
        let Some(sub) = l.chroma else {
            assert_eq!(samples.len(), luma, "{}", l.name);
            return [samples, Vec::new(), Vec::new()];
        };
        let (w, h) = chroma_size(sub);
        let c = w * h;
        assert_eq!(samples.len(), luma + 2 * c, "{}", l.name);
        [
            samples[..luma].to_vec(),
            samples[luma..luma + c].to_vec(),
            samples[luma + c..].to_vec(),
        ]
    }

    /// The planes of `p`, a row after another, and its luma bit depth.
    fn planes_of(p: &Picture<'_>) -> ([Vec<u16>; 3], u32) {
        fn rows(plane: Samples<'_>, stride: usize, (w, h): (usize, usize)) -> Vec<u16> {
            (0..h)
                .flat_map(|r| -> Vec<u16> {
                    match plane {
                        Samples::U8(s) => s[r * stride..][..w].iter().map(|&v| v.into()).collect(),
                        Samples::U16(s) => s[r * stride..][..w].to_vec(),
                    }
                })
                .collect()
        }
        let size = (usize::from(p.width), usize::from(p.height));
        let c = p.layout.chroma_size(size.0, size.1);
        (
            [
                rows(p.y, p.y_stride, size),
                rows(p.cb, p.c_stride, c),
                rows(p.cr, p.c_stride, c),
            ],
            p.luma_shift + 8,
        )
    }

    /// The planes heic-rs gives for the first picture of `hevc`, and its luma bit depth.
    fn decoded_planes(hevc: &[u8]) -> ([Vec<u16>; 3], u32) {
        let (sets, slices) = first_picture(hevc);
        let f = heic_rs::hevc::decode_still(&sets, &slices).unwrap();
        planes_of(&Picture::of_heic(&f, None).unwrap())
    }

    #[test]
    fn every_layout_and_bit_depth_decodes_to_ffmpegs_samples() {
        for l in SAMPLES {
            let (planes, bits) = decoded_planes(l.hevc);
            assert_eq!(bits, l.bits, "{}", l.name);
            assert_eq!(planes, ffmpeg_planes(l), "{}", l.name);
        }
    }

    /// rust_h265, the second decoder, gives ffmpeg's samples too, for the layouts it decodes (4:2:0 and 4:4:4).
    #[test]
    fn the_second_decoder_decodes_to_ffmpegs_samples() {
        for l in SAMPLES
            .iter()
            .filter(|l| matches!(l.chroma, Some((1, 1) | (0, 0))))
        {
            let (sets, slices) = first_picture(l.hevc);
            let f = decode_with_rust_h265(&sets, &slices)
                .unwrap_or_else(|e| panic!("{}: {e:?}", l.name));
            let (planes, bits) = planes_of(&Picture::of_rust_h265(&f, None).unwrap());
            assert_eq!(bits, l.bits, "{}", l.name);
            assert_eq!(planes, ffmpeg_planes(l), "{}", l.name);
        }
    }

    /// The JPEG of each shows the source: its luma and, read back from the RGB by the JFIF equations, its chroma at
    /// the resolution the source has it, both brought to 8 bits and the full range.
    #[test]
    fn every_layout_and_bit_depth_becomes_a_jpeg_of_the_source() {
        for l in SAMPLES {
            let jpg = decode_to_jpeg(l.hevc).unwrap_or_else(|e| panic!("{}: {e:?}", l.name));
            let mut d = jpeg_decoder::Decoder::new(&jpg[..]);
            let pixels = d.decode().unwrap();
            let info = d.info().unwrap();
            assert_eq!((info.width, info.height), (70, 46), "{}", l.name);
            let [y, cb, cr] = ffmpeg_planes(l);
            let scale = f64::from(1u32 << (l.bits - 8));
            let to8 = |v: u16, offset: f64, range: f64, centre: f64| {
                let v = f64::from(v) / scale;
                if l.full {
                    v
                } else {
                    (v - offset) * 255.0 / range + centre
                }
            };
            let (mut luma_err, mut chroma_err) = (0f64, 0f64);
            for r in 0..46 {
                for c in 0..70 {
                    let i = r * 70 + c;
                    let want = to8(y[i], 16.0, 219.0, 0.0);
                    let Some(sub) = l.chroma else {
                        assert_eq!(info.pixel_format, jpeg_decoder::PixelFormat::L8);
                        luma_err += (f64::from(pixels[i]) - want).abs();
                        continue;
                    };
                    let [red, green, blue] = [0, 1, 2].map(|k| f64::from(pixels[i * 3 + k]));
                    luma_err += (0.299 * red + 0.587 * green + 0.114 * blue - want).abs();
                    let k = (r >> sub.1) * chroma_size(sub).0 + (c >> sub.0);
                    let shown_cb = -0.168_736 * red - 0.331_264 * green + 0.5 * blue + 128.0;
                    let shown_cr = 0.5 * red - 0.418_688 * green - 0.081_312 * blue + 128.0;
                    chroma_err += (shown_cb - to8(cb[k], 128.0, 224.0, 128.0)).abs()
                        + (shown_cr - to8(cr[k], 128.0, 224.0, 128.0)).abs();
                }
            }
            let n = 70.0 * 46.0;
            assert!(
                luma_err / n < 4.0,
                "{}: mean luma error {}",
                l.name,
                luma_err / n
            );
            assert!(
                chroma_err / (2.0 * n) < 8.0,
                "{}: mean chroma error {}",
                l.name,
                chroma_err / (2.0 * n)
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

    /// A change to a stream: at a fraction of its length, replace, remove, repeat or insert a byte.
    #[derive(Clone, Debug)]
    enum Damage {
        Replace(f64, u8),
        Remove(f64),
        Repeat(f64),
        Insert(f64, u8),
    }

    fn damage() -> impl proptest::strategy::Strategy<Value = Damage> {
        use proptest::prelude::*;
        prop_oneof![
            (0.0..1.0, any::<u8>()).prop_map(|(at, b)| Damage::Replace(at, b)),
            (0.0..1.0).prop_map(Damage::Remove),
            (0.0..1.0).prop_map(Damage::Repeat),
            (0.0..1.0, any::<u8>()).prop_map(|(at, b)| Damage::Insert(at, b)),
        ]
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a fraction in 0..1 of a short stream's length"
    )]
    fn damaged(sample: &[u8], keep: f64, changes: &[Damage]) -> Vec<u8> {
        let mut data = sample[..(sample.len() as f64 * keep) as usize].to_vec();
        for change in changes {
            if data.is_empty() {
                break;
            }
            let at = |f: f64| ((data.len() as f64 * f) as usize).min(data.len() - 1);
            match *change {
                Damage::Replace(f, b) => {
                    let i = at(f);
                    data[i] = b;
                }
                Damage::Remove(f) => {
                    data.remove(at(f));
                }
                Damage::Repeat(f) => {
                    let i = at(f);
                    data.insert(i, data[i]);
                }
                Damage::Insert(f, b) => data.insert(at(f), b),
            }
        }
        data
    }

    proptest::proptest! {
        #![proptest_config(crate::property_cases(256))]

        /// Any stream of any layout and bit depth, cut short or with any bytes changed, removed, repeated or inserted,
        /// gives an error or a picture, never a panic: on its own and inside a WXGF container (#502).
        #[test]
        fn any_damaged_stream_gives_an_error_or_a_picture(
            sample in 0..SAMPLES.len(),
            keep in 0.0..=1.0f64,
            changes in proptest::collection::vec(damage(), 0..12),
        ) {
            let data = damaged(SAMPLES[sample].hevc, keep, &changes);
            let _ = decode_to_jpeg(&data);
            let mut wxgf = b"wxgf".to_vec();
            wxgf.extend_from_slice(&[0; 16]);
            wxgf.extend_from_slice(&data);
            for (_, stream) in crate::image::wxgf_hevc_candidates(&wxgf) {
                let _ = decode_to_jpeg(&stream);
            }
        }
    }
}
