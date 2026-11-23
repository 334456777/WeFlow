use proptest::prelude::*;

use super::*;

/// A plane of 8-bit samples, row by row.
struct Samples8 {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl Samples8 {
    fn at(&self, x: usize, y: usize) -> u8 {
        self.data[y.min(self.height - 1) * self.width + x.min(self.width - 1)]
    }
}

impl Plane for Samples8 {
    fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn block(&self, x: usize, y: usize, out: &mut [i16; 64]) {
        for (i, o) in out.iter_mut().enumerate() {
            *o = i16::from(self.at(x + i % 8, y + i / 8)) - 128;
        }
    }
}

/// A picture as `jpeg-encoder` takes it: rows at the luma's width, each chroma sample repeated over the luma samples
/// it covers.
struct Widened<'a> {
    luma: &'a Samples8,
    chroma: Option<(&'a Samples8, &'a Samples8, (usize, usize))>,
}

impl jpeg_encoder::ImageBuffer for Widened<'_> {
    fn get_jpeg_color_type(&self) -> jpeg_encoder::JpegColorType {
        if self.chroma.is_some() {
            jpeg_encoder::JpegColorType::Ycbcr
        } else {
            jpeg_encoder::JpegColorType::Luma
        }
    }

    fn width(&self) -> u16 {
        u16::try_from(self.luma.width).unwrap()
    }

    fn height(&self) -> u16 {
        u16::try_from(self.luma.height).unwrap()
    }

    fn fill_buffers(&self, y: u16, buffers: &mut [Vec<u8>; 4]) {
        let (y, w) = (usize::from(y), self.luma.width);
        buffers[0].extend((0..w).map(|x| self.luma.at(x, y)));
        if let Some((cb, cr, (hs, vs))) = self.chroma {
            buffers[1].extend((0..w).map(|x| cb.at(x / hs, y / vs)));
            buffers[2].extend((0..w).map(|x| cr.at(x / hs, y / vs)));
        }
    }
}

/// `jpeg-encoder`'s file for the picture, as WXGF images were converted before this module.
fn reference(picture: Widened<'_>, quality: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, quality);
    match picture.chroma.map(|c| c.2) {
        None => {}
        Some((2, 2)) => encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2),
        Some((2, 1)) => encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_1),
        Some(_) => encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1),
    }
    encoder.encode_image(picture).unwrap();
    out
}

fn plane(width: usize, height: usize, seed: u64) -> Samples8 {
    let mut x = seed | 1;
    let data = (0..width * height)
        .map(|_| {
            // xorshift: noise, with runs of one value now and then so that blocks of zeros turn up too
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 56) as u8
        })
        .collect();
    Samples8 {
        width,
        height,
        data,
    }
}

/// Our file and `jpeg-encoder`'s for a picture of `width` × `height` with chroma sampled by `factors` (gray
/// without).
fn both(
    width: usize,
    height: usize,
    factors: Option<(usize, usize)>,
    quality: u8,
    seed: u64,
) -> (Vec<u8>, Vec<u8>) {
    let luma = plane(width, height, seed);
    let chroma = factors.map(|(hs, vs)| {
        let (cw, ch) = (width.div_ceil(hs), height.div_ceil(vs));
        (
            plane(cw, ch, seed ^ 0x9e37),
            plane(cw, ch, seed ^ 0x79b9),
            (hs, vs),
        )
    });
    let ours = encode(
        &luma,
        chroma.as_ref().map(|(cb, cr, factors)| Chroma {
            cb,
            cr,
            factors: *factors,
        }),
        quality,
    )
    .unwrap();
    let theirs = reference(
        Widened {
            luma: &luma,
            chroma: chroma.as_ref().map(|(cb, cr, f)| (cb, cr, *f)),
        },
        quality,
    );
    (ours, theirs)
}

#[test]
fn files_are_those_jpeg_encoder_writes() {
    for factors in [None, Some((1, 1)), Some((2, 1)), Some((2, 2))] {
        for (width, height) in [(1, 1), (8, 8), (16, 16), (17, 9), (70, 46), (33, 65)] {
            let (ours, theirs) = both(width, height, factors, 92, 7);
            assert!(
                ours == theirs,
                "{width} x {height}, {factors:?}: {} bytes against {}",
                ours.len(),
                theirs.len()
            );
        }
    }
}

#[test]
fn a_picture_without_samples_or_too_large_has_no_file() {
    let empty = Samples8 {
        width: 0,
        height: 4,
        data: Vec::new(),
    };
    assert!(encode(&empty, None, 92).is_none());
    let wide = Samples8 {
        width: 70_000,
        height: 1,
        data: vec![0; 70_000],
    };
    assert!(encode(&wide, None, 92).is_none());
}

#[test]
fn flat_and_extreme_blocks_quantize_alike_on_every_path() {
    let quant = Quant::new(&LUMA_QUANT, 92);
    for block in [
        [-128i16; 64],
        [127; 64],
        [0; 64],
        std::array::from_fn(|i| if i % 2 == 0 { -128 } else { 127 }),
    ] {
        let mut scalar = dct(&block);
        for (i, c) in scalar.iter_mut().enumerate() {
            *c = quant.quantize(*c, i);
        }
        assert_eq!(dct_quantize(&block, &quant), scalar);
    }
}

#[test]
fn magnitudes_are_the_size_and_the_bits_coded() {
    assert_eq!(magnitude(0), (0, 0));
    assert_eq!(magnitude(1), (1, 1));
    assert_eq!(magnitude(-1), (1, 0));
    assert_eq!(magnitude(5), (3, 0b101));
    assert_eq!(magnitude(-5), (3, 0b010));
    assert_eq!(magnitude(2047), (11, 2047));
    assert_eq!(magnitude(-2047), (11, 0));
}

proptest! {
    #![proptest_config(crate::property_cases(64))]

    /// Any size around the MCU's, any sampling of the chroma, any quality.
    #[test]
    fn any_picture_is_the_file_jpeg_encoder_writes(
        width in 1usize..40,
        height in 1usize..40,
        factors in prop_oneof![Just(None), Just(Some((1, 1))), Just(Some((2, 1))), Just(Some((2, 2)))],
        quality in 1u8..=100,
        seed in any::<u64>(),
    ) {
        let (ours, theirs) = both(width, height, factors, quality, seed);
        prop_assert!(ours == theirs, "{} bytes against {}", ours.len(), theirs.len());
    }

    /// Any block: the DCT and the quantization give the same coefficients however they run.
    #[test]
    fn any_block_quantizes_alike_on_every_path(
        samples in proptest::collection::vec(-128i16..=127, 64),
        quality in 1u8..=100,
        chroma in any::<bool>(),
    ) {
        let block: [i16; 64] = samples.try_into().unwrap();
        let quant = Quant::new(if chroma { &CHROMA_QUANT } else { &LUMA_QUANT }, quality);
        let mut scalar = dct(&block);
        for (i, c) in scalar.iter_mut().enumerate() {
            *c = quant.quantize(*c, i);
        }
        prop_assert_eq!(dct_quantize(&block, &quant), scalar);
    }
}
