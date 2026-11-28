//! SILK v3 解码器，与桌面版解码微信语音消息时用的 `silk-wasm` 解码循环一致：数据以 `#!SILK_V3` 开头，前面可能有腾讯
//! 加的一个 `0x02` 字节，之后是以 `i16` 长度开头的数据包，以 `-1` 结束。
use std::ffi::c_void;

#[repr(C)]
struct DecControl {
    api_sample_rate: i32,
    frame_size: i32,
    frames_per_packet: i32,
    more_internal_decoder_frames: i32,
    in_band_fec_offset: i32,
}

// SKP_SILK_SDK_DecControlStruct（vendor/interface/SKP_Silk_control.h）的内存布局：按这个顺序排列的五个 32 位
// 整数（#505）
const _: () = {
    assert!(size_of::<DecControl>() == 20);
    assert!(std::mem::offset_of!(DecControl, api_sample_rate) == 0);
    assert!(std::mem::offset_of!(DecControl, frame_size) == 4);
    assert!(std::mem::offset_of!(DecControl, frames_per_packet) == 8);
    assert!(std::mem::offset_of!(DecControl, more_internal_decoder_frames) == 12);
    assert!(std::mem::offset_of!(DecControl, in_band_fec_offset) == 16);
};

extern "C" {
    fn SKP_Silk_SDK_Get_Decoder_Size(size: *mut i32) -> i32;
    fn SKP_Silk_SDK_InitDecoder(state: *mut c_void) -> i32;
    fn SKP_Silk_SDK_Decode(
        state: *mut c_void,
        ctl: *mut DecControl,
        lost: i32,
        input: *const u8,
        n_bytes: i32,
        out: *mut i16,
        n_out: *mut i16,
    ) -> i32;
}

const HEADER: &[u8] = b"#!SILK_V3";
const MAX_PAYLOAD: usize = 1024;
const MAX_INTERNAL_FRAMES: usize = 5;
/// 解码器每次调用最多写出的采样数：48 kHz 下 20 毫秒的两倍。
const SCRATCH_SAMPLES: i16 = 960 * 2;

/// SILK 数据无法解码的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SilkError {
    /// 数据不以 `#!SILK_V3` 开头（跳过微信可能加的 `0x02` 字节之后）。
    InvalidHeader,
    /// 数据在某个数据包或它的长度中间结束。
    TruncatedPayload,
    /// 数据包的长度为负数，但不是结束标记 -1。
    InvalidPacketLength,
    /// 数据包超过 1024 字节。
    PayloadTooLarge,
    /// 一个数据包里超过 5 帧。
    TooManyInternalFrames,
    /// SDK 返回了这个错误码。
    Sdk(i32),
}

impl std::fmt::Display for SilkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidHeader => write!(f, "not a SILK v3 stream"),
            Self::TruncatedPayload => write!(f, "truncated SILK payload"),
            Self::InvalidPacketLength => write!(f, "invalid SILK packet length"),
            Self::PayloadTooLarge => write!(f, "SILK packet too large"),
            Self::TooManyInternalFrames => write!(f, "too many SILK internal frames"),
            Self::Sdk(code) => write!(f, "SILK decoder error {code}"),
        }
    }
}

impl std::error::Error for SilkError {}

/// 把 SILK v3 数据解码成采样率为 `sample_rate` Hz 的 16 位小端单声道 PCM。
///
/// # Errors
///
/// 数据不是 SILK v3、数据包的长度不对或被截断，或者 SDK 解码失败时，返回对应的 [`SilkError`]。
pub fn decode(silk: &[u8], sample_rate: i32) -> Result<Vec<u8>, SilkError> {
    let mut src = silk;
    if src.first() == Some(&0x02) {
        src = &src[1..];
    }
    if !src.starts_with(HEADER) {
        return Err(SilkError::InvalidHeader);
    }
    src = &src[HEADER.len()..];

    let mut size = 0i32;
    // SAFETY: `size` 是存活的局部变量，SDK 把解码器的大小写到这里。
    let code = unsafe { SKP_Silk_SDK_Get_Decoder_Size(&raw mut size) };
    let size = usize::try_from(size).unwrap_or(0);
    if code != 0 || size == 0 {
        return Err(SilkError::Sdk(code));
    }
    // 用 u64 数组作为存储，使解码器的状态按 8 字节对齐
    let mut state = vec![0u64; size.div_ceil(8)];
    let state_ptr = state.as_mut_ptr().cast::<c_void>();
    // SAFETY: `state` 至少有 SDK 要求的 `size` 字节，并按 8 字节对齐。
    let code = unsafe { SKP_Silk_SDK_InitDecoder(state_ptr) };
    if code != 0 {
        return Err(SilkError::Sdk(code));
    }

    let mut ctl = DecControl {
        api_sample_rate: sample_rate,
        frame_size: 0,
        frames_per_packet: 1,
        more_internal_decoder_frames: 0,
        in_band_fec_offset: 0,
    };
    let mut scratch = vec![0i16; SCRATCH_SAMPLES as usize];
    let mut out: Vec<u8> = Vec::new();
    while !src.is_empty() {
        if src.len() < 2 {
            return Err(SilkError::TruncatedPayload);
        }
        let packet_len = i16::from_le_bytes([src[0], src[1]]);
        src = &src[2..];
        if packet_len == -1 {
            break;
        }
        let Ok(len) = usize::try_from(packet_len) else {
            return Err(SilkError::InvalidPacketLength);
        };
        if len > MAX_PAYLOAD {
            return Err(SilkError::PayloadTooLarge);
        }
        if src.len() < len {
            return Err(SilkError::TruncatedPayload);
        }
        let payload = &src[..len];
        src = &src[len..];
        let mut frames = 0;
        loop {
            if frames >= MAX_INTERNAL_FRAMES {
                return Err(SilkError::TooManyInternalFrames);
            }
            let mut n = SCRATCH_SAMPLES;
            // SAFETY: `state_ptr` 指向上面初始化的解码器，由 `state` 保持存活；`payload` 是 `len` 个有效的字节；
            // `scratch` 能容纳作为容量传入的 `n` 个采样。
            let code = unsafe {
                SKP_Silk_SDK_Decode(
                    state_ptr,
                    &raw mut ctl,
                    0,
                    payload.as_ptr(),
                    i32::from(packet_len),
                    scratch.as_mut_ptr(),
                    &raw mut n,
                )
            };
            if code != 0 {
                return Err(SilkError::Sdk(code));
            }
            for s in &scratch[..usize::try_from(n).unwrap_or(0)] {
                out.extend_from_slice(&s.to_le_bytes());
            }
            frames += 1;
            if ctl.more_internal_decoder_frames == 0 {
                break;
            }
        }
    }
    Ok(out)
}

/// 只用于测试的编码器，调用内置的 SDK，用来生成接近真实的语音测试数据。
#[cfg(any(test, feature = "test-encoder"))]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "一秒的测试音：帧大小和采样值都很小，且不为负"
)]
pub mod testenc {
    use std::ffi::c_void;

    #[repr(C)]
    struct EncControl {
        api_sample_rate: i32,
        max_internal_sample_rate: i32,
        packet_size: i32,
        bit_rate: i32,
        packet_loss_percentage: i32,
        complexity: i32,
        use_in_band_fec: i32,
        use_dtx: i32,
    }

    // SKP_SILK_SDK_EncControlStruct（vendor/interface/SKP_Silk_control.h）的内存布局：按这个顺序排列的八个
    // 32 位整数（#505）
    const _: () = {
        assert!(size_of::<EncControl>() == 32);
        assert!(std::mem::offset_of!(EncControl, api_sample_rate) == 0);
        assert!(std::mem::offset_of!(EncControl, max_internal_sample_rate) == 4);
        assert!(std::mem::offset_of!(EncControl, packet_size) == 8);
        assert!(std::mem::offset_of!(EncControl, bit_rate) == 12);
        assert!(std::mem::offset_of!(EncControl, packet_loss_percentage) == 16);
        assert!(std::mem::offset_of!(EncControl, complexity) == 20);
        assert!(std::mem::offset_of!(EncControl, use_in_band_fec) == 24);
        assert!(std::mem::offset_of!(EncControl, use_dtx) == 28);
    };

    extern "C" {
        fn SKP_Silk_SDK_Get_Encoder_Size(size: *mut i32) -> i32;
        fn SKP_Silk_SDK_InitEncoder(state: *mut c_void, status: *mut EncControl) -> i32;
        fn SKP_Silk_SDK_Encode(
            state: *mut c_void,
            ctl: *const EncControl,
            input: *const i16,
            n: i32,
            out: *mut u8,
            n_out: *mut i16,
        ) -> i32;
    }

    /// 用内置的编码器把一秒的 440 Hz 音调编码成 `.silk` 格式。
    ///
    /// # Panics
    ///
    /// 编码器失败时：`rate` 必须是 SILK 能编码的采样率（8、12、16 或 24 kHz）。
    pub fn encode_tone(rate: i32) -> Vec<u8> {
        let mut size = 0;
        // SAFETY: `size` 是存活的局部变量，SDK 把编码器的大小写到这里。
        assert_eq!(unsafe { SKP_Silk_SDK_Get_Encoder_Size(&raw mut size) }, 0);
        let mut state = vec![0u64; (size as usize).div_ceil(8)];
        let sp = state.as_mut_ptr().cast::<c_void>();
        let mut ctl = EncControl {
            api_sample_rate: 0,
            max_internal_sample_rate: 0,
            packet_size: 0,
            bit_rate: 0,
            packet_loss_percentage: 0,
            complexity: 0,
            use_in_band_fec: 0,
            use_dtx: 0,
        };
        // SAFETY: `state` 至少有 SDK 要求的 `size` 字节，并按 8 字节对齐；`ctl` 是存活的 `EncControl`。
        assert_eq!(unsafe { SKP_Silk_SDK_InitEncoder(sp, &raw mut ctl) }, 0);
        // `InitEncoder` 把编码器的状态写进 `ctl`，所以设置要在它之后再填
        ctl = EncControl {
            api_sample_rate: rate,
            max_internal_sample_rate: 24000,
            packet_size: rate / 50,
            bit_rate: 25000,
            packet_loss_percentage: 0,
            complexity: 1,
            use_in_band_fec: 0,
            use_dtx: 0,
        };
        let frame = (rate / 50) as usize;
        let pcm: Vec<i16> = (0..rate)
            .map(|i| {
                ((f64::from(i) * 440.0 * std::f64::consts::TAU / f64::from(rate)).sin() * 12000.0)
                    as i16
            })
            .collect();
        let mut out = b"#!SILK_V3".to_vec();
        for chunk in pcm.chunks_exact(frame) {
            let mut buf = [0u8; 1250];
            let mut n = buf.len() as i16;
            assert_eq!(
                // SAFETY: `sp` 指向上面初始化的编码器，由 `state` 保持存活；`chunk` 有传入的 `frame` 个采样，
                // `buf` 能容纳作为容量传入的 `n` 个字节。
                unsafe {
                    SKP_Silk_SDK_Encode(
                        sp,
                        &raw const ctl,
                        chunk.as_ptr(),
                        frame as i32,
                        buf.as_mut_ptr(),
                        &raw mut n,
                    )
                },
                0
            );
            if n > 0 {
                out.extend_from_slice(&n.to_le_bytes());
                out.extend_from_slice(&buf[..n as usize]);
            }
        }
        out.extend_from_slice(&(-1i16).to_le_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testenc::encode_tone;
    use super::*;

    #[test]
    fn rejects_non_silk_input() {
        assert_eq!(decode(b"RIFF....", 24000), Err(SilkError::InvalidHeader));
        assert_eq!(decode(b"", 24000), Err(SilkError::InvalidHeader));
    }

    #[test]
    fn header_only_stream_decodes_to_nothing() {
        assert_eq!(decode(b"#!SILK_V3", 24000).unwrap(), Vec::<u8>::new());
        assert_eq!(
            decode(b"\x02#!SILK_V3\xff\xff", 24000).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn truncated_packets_are_errors() {
        assert_eq!(
            decode(b"#!SILK_V3\x05\x00ab", 24000),
            Err(SilkError::TruncatedPayload)
        );
        assert_eq!(
            decode(b"#!SILK_V3\x05", 24000),
            Err(SilkError::TruncatedPayload)
        );
        assert_eq!(
            decode(b"#!SILK_V3\xfe\xff", 24000),
            Err(SilkError::InvalidPacketLength)
        );
    }

    #[test]
    fn round_trips_a_tone_at_the_requested_sample_rate() {
        for rate in [24000, 16000] {
            let silk = encode_tone(rate);
            let pcm = decode(&silk, rate).unwrap();
            let samples: Vec<i16> = pcm
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            assert!(
                (i64::try_from(samples.len()).unwrap() - i64::from(rate)).abs()
                    <= i64::from(rate) / 10,
                "about one second of audio, got {}",
                samples.len()
            );
            let rms = (samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>()
                / f64::from(u32::try_from(samples.len()).unwrap()))
            .sqrt();
            assert!(
                rms > 4000.0 && rms < 12000.0,
                "tone energy survives the codec, rms={rms}"
            );
            // 带腾讯前缀字节的数据解码结果相同
            let mut prefixed = vec![0x02];
            prefixed.extend_from_slice(&silk);
            assert_eq!(decode(&prefixed, rate).unwrap(), pcm);
        }
    }
}
