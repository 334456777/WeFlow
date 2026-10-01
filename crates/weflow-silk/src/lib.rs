//! SILK v3 decoder, matching the `silk-wasm` decode loop the desktop app uses for WeChat voice
//! messages (`#!SILK_V3` header, optional leading `0x02` Tencent byte, `i16` length-prefixed
//! packets terminated by `-1`).
use std::ffi::c_void;

#[repr(C)]
struct DecControl {
    api_sample_rate: i32,
    frame_size: i32,
    frames_per_packet: i32,
    more_internal_decoder_frames: i32,
    in_band_fec_offset: i32,
}

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

#[derive(Debug, PartialEq, Eq)]
pub enum SilkError {
    InvalidHeader,
    TruncatedPayload,
    InvalidPacketLength,
    PayloadTooLarge,
    TooManyInternalFrames,
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

/// Decodes a SILK v3 stream into 16-bit little-endian mono PCM at `sample_rate` Hz.
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
    let code = unsafe { SKP_Silk_SDK_Get_Decoder_Size(&mut size) };
    if code != 0 || size <= 0 {
        return Err(SilkError::Sdk(code));
    }
    // u64 backing store keeps the state 8-byte aligned
    let mut state = vec![0u64; (size as usize + 7) / 8];
    let state_ptr = state.as_mut_ptr() as *mut c_void;
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
    let mut scratch = vec![0i16; 960 * 2];
    let mut out: Vec<u8> = Vec::new();
    while !src.is_empty() {
        if src.len() < 2 {
            return Err(SilkError::TruncatedPayload);
        }
        let len = i16::from_le_bytes([src[0], src[1]]);
        src = &src[2..];
        if len == -1 {
            break;
        }
        if len < 0 {
            return Err(SilkError::InvalidPacketLength);
        }
        let len = len as usize;
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
            let mut n = scratch.len() as i16;
            let code = unsafe {
                SKP_Silk_SDK_Decode(
                    state_ptr,
                    &mut ctl,
                    0,
                    payload.as_ptr(),
                    payload.len() as i32,
                    scratch.as_mut_ptr(),
                    &mut n,
                )
            };
            if code != 0 {
                return Err(SilkError::Sdk(code));
            }
            for s in &scratch[..n.max(0) as usize] {
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

/// Test-only encoder over the vendored SDK, used to build realistic voice fixtures.
#[cfg(any(test, feature = "test-encoder"))]
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

    /// Encodes one second of a 440 Hz tone with the vendored encoder into the `.silk` container.
    pub fn encode_tone(rate: i32) -> Vec<u8> {
        let mut size = 0;
        assert_eq!(unsafe { SKP_Silk_SDK_Get_Encoder_Size(&mut size) }, 0);
        let mut state = vec![0u64; (size as usize + 7) / 8];
        let sp = state.as_mut_ptr() as *mut c_void;
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
        assert_eq!(unsafe { SKP_Silk_SDK_InitEncoder(sp, &mut ctl) }, 0);
        // `InitEncoder` reports the encoder status, so the settings go in afterwards
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
        let pcm: Vec<i16> = (0..rate as usize)
            .map(|i| {
                ((i as f64 * 440.0 * std::f64::consts::TAU / rate as f64).sin() * 12000.0) as i16
            })
            .collect();
        let mut out = b"#!SILK_V3".to_vec();
        for chunk in pcm.chunks_exact(frame) {
            let mut buf = [0u8; 1250];
            let mut n = buf.len() as i16;
            assert_eq!(
                unsafe {
                    SKP_Silk_SDK_Encode(
                        sp,
                        &ctl,
                        chunk.as_ptr(),
                        frame as i32,
                        buf.as_mut_ptr(),
                        &mut n,
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
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            assert!(
                (samples.len() as i64 - rate as i64).abs() <= rate as i64 / 10,
                "about one second of audio, got {}",
                samples.len()
            );
            let rms = (samples.iter().map(|s| (*s as f64).powi(2)).sum::<f64>()
                / samples.len() as f64)
                .sqrt();
            assert!(
                rms > 4000.0 && rms < 12000.0,
                "tone energy survives the codec, rms={rms}"
            );
            // Tencent-prefixed streams decode identically
            let mut prefixed = vec![0x02];
            prefixed.extend_from_slice(&silk);
            assert_eq!(decode(&prefixed, rate).unwrap(), pcm);
        }
    }
}
