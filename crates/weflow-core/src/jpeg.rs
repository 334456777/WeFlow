//! Baseline JPEG of planar YCbCr or gray samples: the files WXGF images are converted to ([`crate::hevc`]).
//!
//! The file is the one `jpeg-encoder` 0.7 writes for the same picture and quality, byte for byte: the JFIF header,
//! the tables of Annex K of the JPEG standard scaled to the quality the way the Independent JPEG Group's software
//! scales them, its integer DCT (`jfdctint.c`, after Loeffler, Ligtenberg and Moschytz: this part is based on the
//! work of the Independent JPEG Group), quantization by a reciprocal, and one interleaved scan. What differs is how
//! it gets there: each block is read straight from its plane, chroma at the resolution the picture has it, rather
//! than from rows widened to the luma's width and sampled again; where the CPU has AVX2, the DCT and the
//! quantization of a block work on eight lanes at once; and on x86_64 the coefficients that are not 0, which the
//! Huffman coding visits, are found sixteen at a time. The tests compare the files with `jpeg-encoder`'s.

/// The samples of one plane, brought to 8 bits.
pub(crate) trait Plane {
    /// Samples across and down.
    fn size(&self) -> (usize, usize);

    /// The 8 × 8 samples from column `x`, row `y`, row by row and less 128, into `out`. Columns and rows past the
    /// plane's last one repeat it: a block may start past the plane.
    fn block(&self, x: usize, y: usize, out: &mut [i16; 64]);
}

/// The chroma planes of a picture, and how often each luma block is sampled against one of theirs: `(2, 2)` for
/// 4:2:0, `(2, 1)` for 4:2:2, `(1, 1)` for 4:4:4.
pub(crate) struct Chroma<'a, P> {
    /// Cb.
    pub(crate) cb: &'a P,
    /// Cr.
    pub(crate) cr: &'a P,
    /// Luma blocks across and down an MCU.
    pub(crate) factors: (usize, usize),
}

/// The JPEG of the picture whose luma is `luma` (gray without `chroma`) at `quality` (1 to 100); `None` when its
/// size cannot be written in a JPEG (no samples, or more than 65,535 across or down).
pub(crate) fn encode<P: Plane>(
    luma: &P,
    chroma: Option<Chroma<'_, P>>,
    quality: u8,
) -> Option<Vec<u8>> {
    let (w, h) = luma.size();
    let (width, height) = (u16::try_from(w).ok()?, u16::try_from(h).ok()?);
    let (hs, vs) = chroma.as_ref().map_or((1, 1), |c| c.factors);
    if w == 0 || h == 0 || !matches!((hs, vs), (1 | 2, 1) | (2, 2)) {
        return None;
    }
    let tables = [
        Quant::new(&LUMA_QUANT, quality),
        Quant::new(&CHROMA_QUANT, quality),
    ];
    let huffman = [
        (
            Huffman::new(&LUMA_DC_LENGTHS, &DC_VALUES),
            Huffman::new(&LUMA_AC_LENGTHS, &LUMA_AC_VALUES),
        ),
        (
            Huffman::new(&CHROMA_DC_LENGTHS, &DC_VALUES),
            Huffman::new(&CHROMA_AC_LENGTHS, &CHROMA_AC_VALUES),
        ),
    ];
    let mut out = Writer::new(w * h / 2);
    out.headers(width, height, chroma.is_some().then_some((hs, vs)), &tables);

    let (mcu_w, mcu_h) = (8 * hs, 8 * vs);
    let mut prev_dc = [0i16; 3];
    let mut block = [0i16; 64];
    for my in 0..h.div_ceil(mcu_h) {
        for mx in 0..w.div_ceil(mcu_w) {
            for v in 0..vs {
                for u in 0..hs {
                    luma.block(mx * mcu_w + u * 8, my * mcu_h + v * 8, &mut block);
                    out.block(&block, &tables[0], &huffman[0], &mut prev_dc[0]);
                }
            }
            if let Some(c) = &chroma {
                for (plane, dc) in [c.cb, c.cr].into_iter().zip(&mut prev_dc[1..]) {
                    plane.block(mx * 8, my * 8, &mut block);
                    out.block(&block, &tables[1], &huffman[1], dc);
                }
            }
        }
    }
    Some(out.finish())
}

/// `ZIGZAG[i]`: where in the block (row by row) the `i`th coefficient in zigzag order is (figure A.6).
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// The coefficients of a block (row by row) in zigzag order. [`ZIGZAG`] written out, an index a constant, so that
/// each coefficient is one load and one store with no table of indices to read and no loop.
fn zigzag(c: &[i16; 64]) -> [i16; 64] {
    macro_rules! gather {
        ($($i:literal)*) => { [$(c[$i]),*] };
    }
    gather!(
        0 1 8 16 9 2 3 10 17 24 32 25 18 11 4 5 12 19 26 33 40 48 41 34 27 20 13 6 7 14 21 28
        35 42 49 56 57 50 43 36 29 22 15 23 30 37 44 51 58 59 52 45 38 31 39 46 53 60 61 54 47 55 62 63
    )
}

/// The luminance quantization table of Annex K.1, row by row.
const LUMA_QUANT: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];

/// The chrominance quantization table of Annex K.1, row by row.
const CHROMA_QUANT: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

/// The Huffman tables of Annex K.3: how many codes of each length from 1 to 16 bits, and the values they code.
const LUMA_DC_LENGTHS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const CHROMA_DC_LENGTHS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_VALUES: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const LUMA_AC_LENGTHS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const LUMA_AC_VALUES: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];
const CHROMA_AC_LENGTHS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const CHROMA_AC_VALUES: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0,
    0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];

/// Bits of the reciprocals [`Quant`] divides by.
const RECIPROCAL_BITS: u32 = 15;

/// A quantization table at a quality: the values written in the file, and how each coefficient is divided by eight
/// times its value (the DCT leaves the coefficients eight times too large).
struct Quant {
    /// Row by row.
    written: [u8; 64],
    reciprocals: [i32; 64],
    corrections: [i32; 64],
}

impl Quant {
    /// `table` scaled to `quality` as the IJG's software scales it.
    fn new(table: &[u16; 64], quality: u8) -> Self {
        let quality = u32::from(quality.clamp(1, 100));
        let scale = if quality < 50 {
            5000 / quality
        } else {
            200 - quality * 2
        };
        let mut q = Self {
            written: [0; 64],
            reciprocals: [0; 64],
            corrections: [0; 64],
        };
        for (i, &v) in table.iter().enumerate() {
            let v = ((u32::from(v) * scale + 50) / 100).clamp(1, 255);
            q.written[i] = u8::try_from(v).unwrap_or(u8::MAX);
            (q.reciprocals[i], q.corrections[i]) = reciprocal(v << 3);
        }
        q
    }

    /// Coefficient `i` (row by row) of a block quantized: rounded to the nearest multiple, by the reciprocal.
    fn quantize(&self, value: i16, i: usize) -> i16 {
        let v = i32::from(value);
        let q = ((v.abs() + self.corrections[i]) * self.reciprocals[i]) >> RECIPROCAL_BITS;
        #[expect(clippy::cast_possible_truncation, reason = "below 2^15")]
        let q = q as i16;
        if v < 0 {
            -q
        } else {
            q
        }
    }
}

/// The reciprocal of `divisor` (above 1) in [`RECIPROCAL_BITS`] bits, and what to add before multiplying by it so
/// that the product rounds to the nearest.
fn reciprocal(divisor: u32) -> (i32, i32) {
    let (mut r, fraction) = (
        (1 << RECIPROCAL_BITS) / divisor,
        (1 << RECIPROCAL_BITS) % divisor,
    );
    let mut correction = divisor / 2;
    if fraction != 0 {
        if fraction <= correction {
            correction += 1;
        } else {
            r += 1;
        }
    }
    // both below 2^15
    (
        i32::try_from(r).unwrap_or(i32::MAX),
        i32::try_from(correction).unwrap_or(i32::MAX),
    )
}

/// The codes of a Huffman table: `(length, code)` of each value.
struct Huffman([(u8, u16); 256]);

impl Huffman {
    /// The canonical codes of `values`, of the `lengths` given (Annex C).
    fn new(lengths: &[u8; 16], values: &[u8]) -> Self {
        let mut codes = [(0, 0); 256];
        let (mut code, mut k) = (0u16, 0);
        for (bits, &n) in (1u8..).zip(lengths) {
            for _ in 0..n {
                codes[usize::from(values[k])] = (bits, code);
                code += 1;
                k += 1;
            }
            code <<= 1;
        }
        Self(codes)
    }
}

/// The table numbers and lengths of the four Huffman tables, as written.
const HUFFMAN_SEGMENTS: [(u8, &[u8; 16], &[u8]); 4] = [
    (0x00, &LUMA_DC_LENGTHS, &DC_VALUES),
    (0x10, &LUMA_AC_LENGTHS, &LUMA_AC_VALUES),
    (0x01, &CHROMA_DC_LENGTHS, &DC_VALUES),
    (0x11, &CHROMA_AC_LENGTHS, &CHROMA_AC_VALUES),
];

/// The file being written: the segments, then the entropy-coded scan bit by bit.
struct Writer {
    out: Vec<u8>,
    /// Bits not yet written, the last `bits` of them.
    pending: u64,
    bits: u32,
}

impl Writer {
    fn new(capacity: usize) -> Self {
        Self {
            out: Vec::with_capacity(capacity),
            pending: 0,
            bits: 0,
        }
    }

    fn marker(&mut self, marker: u8) {
        self.out.extend([0xff, marker]);
    }

    fn u16(&mut self, v: u16) {
        self.out.extend(v.to_be_bytes());
    }

    /// Everything before the scan: SOI, the JFIF header, SOF0, both quantization tables, the Huffman tables and SOS.
    /// `subsampling` is the luma's factors of a colour picture, `None` for gray.
    fn headers(
        &mut self,
        width: u16,
        height: u16,
        subsampling: Option<(usize, usize)>,
        tables: &[Quant; 2],
    ) {
        self.marker(0xd8);
        // JFIF 1.02, no density unit, aspect ratio 1:1, no thumbnail
        self.marker(0xe0);
        self.out.extend([
            0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x02, 0x00, 0x00, 0x01, 0x00, 0x01,
            0x00, 0x00,
        ]);
        // (id, sampling factors, quantization and Huffman tables) of each component
        let components: &[(u8, u8, u8)] = match subsampling {
            None => &[(0, 0x11, 0)],
            Some((2, 2)) => &[(0, 0x22, 0), (1, 0x11, 1), (2, 0x11, 1)],
            Some((2, _)) => &[(0, 0x21, 0), (1, 0x11, 1), (2, 0x11, 1)],
            Some(_) => &[(0, 0x11, 0), (1, 0x11, 1), (2, 0x11, 1)],
        };
        let count = u8::try_from(components.len()).unwrap_or(u8::MAX);
        self.marker(0xc0);
        self.u16(8 + 3 * u16::from(count));
        self.out.push(8);
        self.u16(height);
        self.u16(width);
        self.out.push(count);
        for &(id, factors, table) in components {
            self.out.extend([id, factors, table]);
        }
        for (dest, q) in (0u8..).zip(tables) {
            self.marker(0xdb);
            self.u16(67);
            self.out.push(dest);
            self.out.extend(ZIGZAG.map(|i| q.written[i]));
        }
        let segments = if components.len() == 1 { 2 } else { 4 };
        for (class, lengths, values) in &HUFFMAN_SEGMENTS[..segments] {
            self.marker(0xc4);
            // 2 + 1 + 16 + at most 162 values
            self.u16(u16::try_from(19 + values.len()).unwrap_or(u16::MAX));
            self.out.push(*class);
            self.out.extend_from_slice(*lengths);
            self.out.extend_from_slice(values);
        }
        self.marker(0xda);
        self.u16(6 + 2 * u16::from(count));
        self.out.push(count);
        for &(id, _, table) in components {
            self.out.extend([id, table << 4 | table]);
        }
        self.out.extend([0, 63, 0]);
    }

    /// Appends the last `size` bits of `value` (at most 32) to the scan, a 0 after each `0xff` byte.
    #[inline]
    fn put(&mut self, value: u32, size: u32) {
        self.pending = self.pending << size | u64::from(value);
        self.bits += size;
        if self.bits >= 32 {
            self.bits -= 32;
            #[expect(clippy::cast_possible_truncation, reason = "the 32 bits wanted")]
            let word = (self.pending >> self.bits) as u32;
            // a byte that is 0xff: its top bit set, and the top bit lost when 1 is added to it
            if word & !word.wrapping_add(0x0101_0101) & 0x8080_8080 == 0 {
                self.out.extend(word.to_be_bytes());
            } else {
                self.stuffed(word);
            }
        }
    }

    /// [`Writer::put`]'s 32 bits `word`, of which a byte is `0xff`: each such byte followed by a 0. Kept out of
    /// `put`, which is in the loop over every coefficient, for the one word in some tens that needs it.
    #[cold]
    fn stuffed(&mut self, word: u32) {
        for byte in word.to_be_bytes() {
            self.out.push(byte);
            if byte == 0xff {
                self.out.push(0);
            }
        }
    }

    /// Codes one block of samples (less 128) of a component with `quant` and its Huffman tables (DC, AC), after a
    /// block of the component whose DC coefficient was `prev_dc`.
    fn block(
        &mut self,
        samples: &[i16; 64],
        quant: &Quant,
        (dc, ac): &(Huffman, Huffman),
        prev_dc: &mut i16,
    ) {
        let zz = zigzag(&dct_quantize(samples, quant));
        let diff = zz[0] - *prev_dc;
        *prev_dc = zz[0];
        let (size, bits) = magnitude(diff);
        let (len, code) = dc.0[usize::from(size)];
        self.put(
            u32::from(code) << size | u32::from(bits),
            u32::from(len) + u32::from(size),
        );
        // the AC coefficients that are not 0, from the first
        let mut nonzero = nonzero(&zz) & !1;
        let mut last = 0;
        while nonzero != 0 {
            let i = nonzero.trailing_zeros();
            nonzero &= nonzero - 1;
            let mut run = i - last - 1;
            while run > 15 {
                let (len, code) = ac.0[0xf0];
                self.put(u32::from(code), u32::from(len));
                run -= 16;
            }
            let (size, bits) = magnitude(zz[i as usize]);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a run below 16, a size below 16"
            )]
            let symbol = (run << 4) as u8 | size;
            let (len, code) = ac.0[usize::from(symbol)];
            self.put(
                u32::from(code) << size | u32::from(bits),
                u32::from(len) + u32::from(size),
            );
            last = i;
        }
        if last < 63 {
            let (len, code) = ac.0[0];
            self.put(u32::from(code), u32::from(len));
        }
    }

    /// The scan's last bits padded with 1s to a whole byte, and EOI.
    fn finish(mut self) -> Vec<u8> {
        let pad = (8 - self.bits % 8) % 8;
        self.put((1 << pad) - 1, pad);
        while self.bits >= 8 {
            self.bits -= 8;
            #[expect(clippy::cast_possible_truncation, reason = "one byte")]
            let byte = (self.pending >> self.bits) as u8;
            self.out.push(byte);
            if byte == 0xff {
                self.out.push(0);
            }
        }
        self.marker(0xd9);
        self.out
    }
}

/// Bit `i` set where `zz[i]` is not 0.
#[cfg(target_arch = "x86_64")]
fn nonzero(zz: &[i16; 64]) -> u64 {
    // SAFETY: SSE2 is part of x86_64: every CPU that runs this code has it.
    unsafe { nonzero_sse2(zz) }
}

/// [`nonzero`] sixteen coefficients at a time: compared with 0, and their bytes' top bits gathered.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
fn nonzero_sse2(zz: &[i16; 64]) -> u64 {
    use std::arch::x86_64::{
        _mm_cmpeq_epi16, _mm_loadu_si128, _mm_movemask_epi8, _mm_packs_epi16, _mm_setzero_si128,
    };
    let zero = _mm_setzero_si128();
    let (sixteens, _) = zz.as_chunks::<16>();
    let mut mask = 0u64;
    for (k, sixteen) in sixteens.iter().enumerate() {
        let (low, high) = sixteen.split_at(8);
        // SAFETY: each half is eight `i16`s, the 16 bytes an unaligned load reads.
        let (low, high) = unsafe {
            (
                _mm_loadu_si128(low.as_ptr().cast()),
                _mm_loadu_si128(high.as_ptr().cast()),
            )
        };
        // every lane of the comparisons 0 or -1, which the saturating pack keeps
        let zeros = _mm_packs_epi16(_mm_cmpeq_epi16(low, zero), _mm_cmpeq_epi16(high, zero));
        let zeros = _mm_movemask_epi8(zeros).cast_unsigned() & 0xffff;
        mask |= u64::from(!zeros & 0xffff) << (16 * k);
    }
    mask
}

/// Bit `i` set where `zz[i]` is not 0.
#[cfg(not(target_arch = "x86_64"))]
fn nonzero(zz: &[i16; 64]) -> u64 {
    nonzero_each(zz)
}

/// [`nonzero`] a coefficient at a time.
#[cfg(any(test, not(target_arch = "x86_64")))]
fn nonzero_each(zz: &[i16; 64]) -> u64 {
    zz.iter()
        .enumerate()
        .fold(0u64, |m, (i, &v)| m | u64::from(v != 0) << i)
}

/// The size category of a coefficient and its bits as coded: the low bits of the value, of the value less one when
/// it is negative.
fn magnitude(v: i16) -> (u8, u16) {
    let size = 16 - v.unsigned_abs().leading_zeros();
    #[expect(clippy::cast_possible_truncation, reason = "at most 16")]
    let size = size as u8;
    let mask = u16::try_from((1u32 << size) - 1).unwrap_or(u16::MAX);
    let bits = (v - i16::from(v < 0)).cast_unsigned() & mask;
    (size, bits)
}

/// The DCT of `samples` (row by row), quantized with `quant`, row by row.
fn dct_quantize(samples: &[i16; 64], quant: &Quant) -> [i16; 64] {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        // SAFETY: the CPU has AVX2, which `avx2::dct_quantize` is compiled for
        return unsafe { avx2::dct_quantize(samples, quant) };
    }
    let mut coefficients = dct(samples);
    for (i, c) in coefficients.iter_mut().enumerate() {
        *c = quant.quantize(*c, i);
    }
    coefficients
}

/// The fixed-point constants of the integer DCT, in 13 bits: `FIX_x` is `x * 2^13` rounded.
const CONST_BITS: i32 = 13;
/// The extra bits the first pass keeps.
const PASS1_BITS: i32 = 2;
const FIX_0_298631336: i32 = 2446;
const FIX_0_390180644: i32 = 3196;
const FIX_0_541196100: i32 = 4433;
const FIX_0_765366865: i32 = 6270;
const FIX_0_899976223: i32 = 7373;
const FIX_1_175875602: i32 = 9633;
const FIX_1_501321110: i32 = 12299;
const FIX_1_847759065: i32 = 15137;
const FIX_1_961570560: i32 = 16069;
const FIX_2_053119869: i32 = 16819;
const FIX_2_562915447: i32 = 20995;
const FIX_3_072711026: i32 = 25172;

/// `x / 2^n`, rounded half up.
fn descale(x: i32, n: i32) -> i32 {
    (x + (1 << (n - 1))) >> n
}

/// One pass of the integer DCT over eight values `s` (a row, then a column), its outputs scaled down by `shift`
/// bits where a product is taken and by `even_shift` where not (`even_shift` negative: scaled up).
fn dct_1d(s: [i32; 8], shift: i32, even_shift: i32) -> [i32; 8] {
    let (tmp0, tmp7) = (s[0] + s[7], s[0] - s[7]);
    let (tmp1, tmp6) = (s[1] + s[6], s[1] - s[6]);
    let (tmp2, tmp5) = (s[2] + s[5], s[2] - s[5]);
    let (tmp3, tmp4) = (s[3] + s[4], s[3] - s[4]);
    let (tmp10, tmp13) = (tmp0 + tmp3, tmp0 - tmp3);
    let (tmp11, tmp12) = (tmp1 + tmp2, tmp1 - tmp2);
    let even = |x: i32| {
        if even_shift < 0 {
            x << -even_shift
        } else {
            descale(x, even_shift)
        }
    };
    let mut out = [0i32; 8];
    out[0] = even(tmp10 + tmp11);
    out[4] = even(tmp10 - tmp11);
    let z1 = (tmp12 + tmp13) * FIX_0_541196100;
    out[2] = descale(z1 + tmp13 * FIX_0_765366865, shift);
    out[6] = descale(z1 - tmp12 * FIX_1_847759065, shift);
    let (z1, z2, z3, z4) = (tmp4 + tmp7, tmp5 + tmp6, tmp4 + tmp6, tmp5 + tmp7);
    let z5 = (z3 + z4) * FIX_1_175875602;
    let (tmp4, tmp5) = (tmp4 * FIX_0_298631336, tmp5 * FIX_2_053119869);
    let (tmp6, tmp7) = (tmp6 * FIX_3_072711026, tmp7 * FIX_1_501321110);
    let (z1, z2) = (z1 * -FIX_0_899976223, z2 * -FIX_2_562915447);
    let z3 = z3 * -FIX_1_961570560 + z5;
    let z4 = z4 * -FIX_0_390180644 + z5;
    out[7] = descale(tmp4 + z1 + z3, shift);
    out[5] = descale(tmp5 + z2 + z4, shift);
    out[3] = descale(tmp6 + z2 + z3, shift);
    out[1] = descale(tmp7 + z1 + z4, shift);
    out
}

/// The integer DCT of a block (row by row), its coefficients eight times a true DCT's.
fn dct(samples: &[i16; 64]) -> [i16; 64] {
    let mut rows = [[0i32; 8]; 8];
    for (r, row) in rows.iter_mut().enumerate() {
        let s = std::array::from_fn(|c| i32::from(samples[r * 8 + c]));
        *row = dct_1d(s, CONST_BITS - PASS1_BITS, -PASS1_BITS);
    }
    let mut out = [0i16; 64];
    for c in 0..8 {
        let column = dct_1d(
            std::array::from_fn(|r| rows[r][c]),
            CONST_BITS + PASS1_BITS,
            PASS1_BITS,
        );
        for (r, v) in column.into_iter().enumerate() {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "below 2^15 for 8-bit samples"
            )]
            let v = v as i16;
            out[r * 8 + c] = v;
        }
    }
    out
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    //! [`super::dct_quantize`] on eight lanes: the arithmetic of [`super::dct`] and [`super::Quant::quantize`]
    //! exactly, eight rows or columns at once.

    use std::arch::x86_64::{
        __m256i, _mm256_abs_epi32, _mm256_add_epi32, _mm256_cvtepi16_epi32, _mm256_loadu_si256,
        _mm256_mullo_epi32, _mm256_packs_epi32, _mm256_permute2x128_si256,
        _mm256_permute4x64_epi64, _mm256_set1_epi32, _mm256_sign_epi32, _mm256_slli_epi32,
        _mm256_srai_epi32, _mm256_storeu_si256, _mm256_sub_epi32, _mm256_unpackhi_epi32,
        _mm256_unpackhi_epi64, _mm256_unpacklo_epi32, _mm256_unpacklo_epi64, _mm_loadu_si128,
    };

    use super::{
        Quant, CONST_BITS, FIX_0_298631336, FIX_0_390180644, FIX_0_541196100, FIX_0_765366865,
        FIX_0_899976223, FIX_1_175875602, FIX_1_501321110, FIX_1_847759065, FIX_1_961570560,
        FIX_2_053119869, FIX_2_562915447, FIX_3_072711026, PASS1_BITS, RECIPROCAL_BITS,
    };

    /// `x / 2^N`, rounded half up, in each lane.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn descale<const N: i32>(x: __m256i) -> __m256i {
        _mm256_srai_epi32::<N>(_mm256_add_epi32(x, _mm256_set1_epi32(1 << (N - 1))))
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    fn mul(x: __m256i, k: i32) -> __m256i {
        _mm256_mullo_epi32(x, _mm256_set1_epi32(k))
    }

    /// The 8 × 8 lanes of `v` transposed: lane `j` of `v[i]` becomes lane `i` of `v[j]`.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn transpose(v: [__m256i; 8]) -> [__m256i; 8] {
        let a0 = _mm256_unpacklo_epi32(v[0], v[1]);
        let a1 = _mm256_unpackhi_epi32(v[0], v[1]);
        let a2 = _mm256_unpacklo_epi32(v[2], v[3]);
        let a3 = _mm256_unpackhi_epi32(v[2], v[3]);
        let a4 = _mm256_unpacklo_epi32(v[4], v[5]);
        let a5 = _mm256_unpackhi_epi32(v[4], v[5]);
        let a6 = _mm256_unpacklo_epi32(v[6], v[7]);
        let a7 = _mm256_unpackhi_epi32(v[6], v[7]);
        let b0 = _mm256_unpacklo_epi64(a0, a2);
        let b1 = _mm256_unpackhi_epi64(a0, a2);
        let b2 = _mm256_unpacklo_epi64(a1, a3);
        let b3 = _mm256_unpackhi_epi64(a1, a3);
        let b4 = _mm256_unpacklo_epi64(a4, a6);
        let b5 = _mm256_unpackhi_epi64(a4, a6);
        let b6 = _mm256_unpacklo_epi64(a5, a7);
        let b7 = _mm256_unpackhi_epi64(a5, a7);
        [
            _mm256_permute2x128_si256::<0x20>(b0, b4),
            _mm256_permute2x128_si256::<0x20>(b1, b5),
            _mm256_permute2x128_si256::<0x20>(b2, b6),
            _mm256_permute2x128_si256::<0x20>(b3, b7),
            _mm256_permute2x128_si256::<0x31>(b0, b4),
            _mm256_permute2x128_si256::<0x31>(b1, b5),
            _mm256_permute2x128_si256::<0x31>(b2, b6),
            _mm256_permute2x128_si256::<0x31>(b3, b7),
        ]
    }

    /// [`super::dct_1d`] in each lane: `s[k]` holds the `k`th value of eight rows (or columns). `FIRST`: the first
    /// pass (the even outputs scaled up by [`PASS1_BITS`], the others down by `CONST_BITS - PASS1_BITS`); otherwise
    /// the second (scaled down by `PASS1_BITS` and `CONST_BITS + PASS1_BITS`).
    #[inline]
    #[target_feature(enable = "avx2")]
    fn dct_1d<const FIRST: bool>(s: [__m256i; 8]) -> [__m256i; 8] {
        let add = _mm256_add_epi32;
        let sub = _mm256_sub_epi32;
        let odd = |x: __m256i| {
            if FIRST {
                descale::<{ CONST_BITS - PASS1_BITS }>(x)
            } else {
                descale::<{ CONST_BITS + PASS1_BITS }>(x)
            }
        };
        let even = |x: __m256i| {
            if FIRST {
                _mm256_slli_epi32::<PASS1_BITS>(x)
            } else {
                descale::<PASS1_BITS>(x)
            }
        };
        let (tmp0, tmp7) = (add(s[0], s[7]), sub(s[0], s[7]));
        let (tmp1, tmp6) = (add(s[1], s[6]), sub(s[1], s[6]));
        let (tmp2, tmp5) = (add(s[2], s[5]), sub(s[2], s[5]));
        let (tmp3, tmp4) = (add(s[3], s[4]), sub(s[3], s[4]));
        let (tmp10, tmp13) = (add(tmp0, tmp3), sub(tmp0, tmp3));
        let (tmp11, tmp12) = (add(tmp1, tmp2), sub(tmp1, tmp2));
        let z1 = mul(add(tmp12, tmp13), FIX_0_541196100);
        let out2 = odd(add(z1, mul(tmp13, FIX_0_765366865)));
        let out6 = odd(sub(z1, mul(tmp12, FIX_1_847759065)));
        let (z1, z2) = (add(tmp4, tmp7), add(tmp5, tmp6));
        let (z3, z4) = (add(tmp4, tmp6), add(tmp5, tmp7));
        let z5 = mul(add(z3, z4), FIX_1_175875602);
        let (tmp4, tmp5) = (mul(tmp4, FIX_0_298631336), mul(tmp5, FIX_2_053119869));
        let (tmp6, tmp7) = (mul(tmp6, FIX_3_072711026), mul(tmp7, FIX_1_501321110));
        let (z1, z2) = (mul(z1, -FIX_0_899976223), mul(z2, -FIX_2_562915447));
        let z3 = add(mul(z3, -FIX_1_961570560), z5);
        let z4 = add(mul(z4, -FIX_0_390180644), z5);
        [
            even(add(tmp10, tmp11)),
            odd(add(add(tmp7, z1), z4)),
            out2,
            odd(add(add(tmp6, z2), z3)),
            even(sub(tmp10, tmp11)),
            odd(add(add(tmp5, z2), z4)),
            out6,
            odd(add(add(tmp4, z1), z3)),
        ]
    }

    /// [`super::dct_quantize`].
    #[target_feature(enable = "avx2")]
    pub(super) fn dct_quantize(samples: &[i16; 64], quant: &Quant) -> [i16; 64] {
        // rows: lane `c` of `rows[r]` is sample `(r, c)`
        let rows: [__m256i; 8] = std::array::from_fn(|r| {
            // SAFETY: the 8 samples of row `r` are in `samples`, and the load needs no alignment
            _mm256_cvtepi16_epi32(unsafe { _mm_loadu_si128(samples[r * 8..].as_ptr().cast()) })
        });
        // the first pass works along each row: `columns[c]` holds sample `(r, c)` of every row `r`
        let pass1 = dct_1d::<true>(transpose(rows));
        // and the second along each column: lane `c` of `rows[r]` is coefficient `(r, c)`
        let out = dct_1d::<false>(transpose(pass1));
        let mut coefficients = [0i16; 64];
        for (r, pair) in out.as_chunks::<2>().0.iter().enumerate() {
            let quantized: [__m256i; 2] = std::array::from_fn(|k| {
                let at = (2 * r + k) * 8;
                // SAFETY: eight `i32` of each table from `at`, which ends at 64; the load needs no alignment
                let (reciprocal, correction) = unsafe {
                    (
                        _mm256_loadu_si256(quant.reciprocals[at..].as_ptr().cast()),
                        _mm256_loadu_si256(quant.corrections[at..].as_ptr().cast()),
                    )
                };
                let v = pair[k];
                let q = _mm256_srai_epi32::<{ RECIPROCAL_BITS.cast_signed() }>(_mm256_mullo_epi32(
                    _mm256_add_epi32(_mm256_abs_epi32(v), correction),
                    reciprocal,
                ));
                // the sign of the coefficient (0 stays 0)
                _mm256_sign_epi32(q, v)
            });
            // packed lane by lane within each 128-bit half: put the two rows back in order
            let packed = _mm256_permute4x64_epi64::<0b11_01_10_00>(_mm256_packs_epi32(
                quantized[0],
                quantized[1],
            ));
            // SAFETY: sixteen `i16` from `16 * r`, which ends at 64; the store needs no alignment
            unsafe { _mm256_storeu_si256(coefficients[16 * r..].as_mut_ptr().cast(), packed) };
        }
        coefficients
    }
}

#[cfg(test)]
mod tests;
