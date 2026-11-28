//! 在 AVX2 寄存器的四个 64 位通道里同时计算四条消息的 SHA-512（多缓冲区哈希），
//! 用于每次读页时校验的 HMAC-SHA512（#1101），以及与页密钥派生（[`crate::kdf`]）共用的 HMAC 部件。
//!
//! SHA-512 是一条链：80 轮中的每一轮都要等前一轮算完，标量计算时 CPU 的大部分时间花在等待上。
//! 这里改为在 256 位寄存器的各个通道里计算四条长度相同、互不相关的消息，每条指令同时完成四条消息的一步；
//! AVX2 没有 64 位循环移位，所以一次循环移位用两次移位和一次或运算完成。CPU 没有 AVX2 时，调用方用 `sha2`
//! 逐条计算，测试也以 `sha2` 的结果为对照。

use sha2::block_api::compress512;
use sha2::{Digest, Sha512};

/// [`hmac4`] 一次计算的消息条数。
pub(crate) const LANES: usize = 4;
/// SHA-512 的块大小（字节）。
pub(crate) const BLOCK: usize = 128;
/// SHA-512 的输出长度（字节）。
pub(crate) const OUT: usize = 64;

/// SHA-512 的初始哈希值（FIPS 180-4，5.3.5 节）。
const IV: [u64; 8] = [
    0x6a09_e667_f3bc_c908,
    0xbb67_ae85_84ca_a73b,
    0x3c6e_f372_fe94_f82b,
    0xa54f_f53a_5f1d_36f1,
    0x510e_527f_ade6_82d1,
    0x9b05_688c_2b3e_6c1f,
    0x1f83_d9ab_fb41_bd6b,
    0x5be0_cd19_137e_2179,
];

/// 只写入一次密钥的 HMAC-SHA512（RFC 2104）：处理完内部和外部填充密钥后的两个 SHA-512 状态。
#[derive(Clone, Copy, Debug)]
pub(crate) struct HmacKey {
    pub(crate) inner: [u64; 8],
    pub(crate) outer: [u64; 8],
}

impl HmacKey {
    /// `key` 对应的状态：比一个块长的密钥先做哈希，较短的用 0 填充。
    pub(crate) fn new(key: &[u8]) -> Self {
        let mut padded = [0u8; BLOCK];
        if key.len() > BLOCK {
            padded[..OUT].copy_from_slice(&Sha512::digest(key));
        } else {
            padded[..key.len()].copy_from_slice(key);
        }
        let keyed = |pad: u8| {
            let mut block = padded;
            for b in &mut block {
                *b ^= pad;
            }
            let mut state = IV;
            compress512(&mut state, &[block]);
            state
        };
        Self {
            inner: keyed(0x36),
            outer: keyed(0x5c),
        }
    }
}

/// SHA-512 输出的八个大端字。
pub(crate) fn words(bytes: &[u8; OUT]) -> [u64; 8] {
    let mut out = [0u64; 8];
    for (w, chunk) in out.iter_mut().zip(bytes.as_chunks::<8>().0) {
        *w = u64::from_be_bytes(*chunk);
    }
    out
}

/// 把 `state` 的八个字按大端写进 `out` 的前 64 字节。
pub(crate) fn put_words(out: &mut [u8], state: &[u64; 8]) {
    for (chunk, w) in out.as_chunks_mut::<8>().0.iter_mut().zip(state) {
        *chunk = w.to_be_bytes();
    }
}

/// 用 `key` 计算四条长度相同的消息的 HMAC-SHA512 标签，第 `l` 条消息是 `bodies[l]` 后接 `tails[l]`；CPU 没有
/// AVX2 时为 `None`，这时消息要逐条计算。
///
/// # Panics
///
/// `bodies` 或 `tails` 的长度不全相同时 panic。
pub(crate) fn hmac4(
    key: &HmacKey,
    bodies: [&[u8]; LANES],
    tails: [&[u8]; LANES],
) -> Option<[[u8; OUT]; LANES]> {
    assert!(
        bodies.iter().all(|b| b.len() == bodies[0].len())
            && tails.iter().all(|t| t.len() == tails[0].len()),
        "messages hashed side by side have the same length"
    );
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        // SAFETY: CPU 支持 AVX2，`avx2::hmac4` 正是为它编译的
        return Some(unsafe { avx2::hmac4(key, bodies, tails) });
    }
    // 其他架构上由调用方逐条计算（aarch64 上用 CPU 的 SHA-512 指令）
    #[cfg(not(target_arch = "x86_64"))]
    let _ = key;
    None
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use std::arch::x86_64::*;

    use super::{HmacKey, BLOCK, LANES, OUT};

    /// SHA-512 的轮常量（FIPS 180-4，4.2.3 节）。
    #[rustfmt::skip]
    const K: [u64; 80] = [
        0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc,
        0x3956c25bf348b538, 0x59f111f1b605d019, 0x923f82a4af194f9b, 0xab1c5ed5da6d8118,
        0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
        0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694,
        0xe49b69c19ef14ad2, 0xefbe4786384f25e3, 0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65,
        0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
        0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4,
        0xc6e00bf33da88fc2, 0xd5a79147930aa725, 0x06ca6351e003826f, 0x142929670a0e6e70,
        0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
        0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b,
        0xa2bfe8a14cf10364, 0xa81a664bbc423001, 0xc24b8b70d0f89791, 0xc76c51a30654be30,
        0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
        0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8,
        0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb, 0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3,
        0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
        0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b,
        0xca273eceea26619c, 0xd186b8c721c0c207, 0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178,
        0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
        0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c,
        0x4cc5d4becb3e42b6, 0x597f299cfc657e2a, 0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
    ];

    /// 四个 SHA-512 状态：第 `i` 个元素存放四个状态的第 `i` 个字，第 `l` 个通道属于第 `l` 条消息。
    type State = [__m256i; 8];

    /// [`super::hmac4`] 的实现，长度已经检查过。
    #[target_feature(enable = "avx2")]
    pub(super) fn hmac4(
        key: &HmacKey,
        bodies: [&[u8]; LANES],
        tails: [&[u8]; LANES],
    ) -> [[u8; OUT]; LANES] {
        let (body, tail) = (bodies[0].len(), tails[0].len());
        // 内部哈希：完全落在 `bodies` 里的块直接从中读取，其余部分连同 `tails` 和填充（一个 1 位、若干 0，
        // 以及密钥块加消息的位长度）复制出来
        let mut state = splat(&key.inner);
        let whole = body / BLOCK;
        for b in 0..whole {
            let at = b * BLOCK;
            compress(&mut state, bodies.map(|m| block_at(m, at)));
        }
        let rest = body - whole * BLOCK + tail;
        let last = if rest + 1 + 16 <= BLOCK {
            BLOCK
        } else {
            2 * BLOCK
        };
        let bits = ((BLOCK + body + tail) as u128 * 8).to_be_bytes();
        let mut ends = [[0u8; 2 * BLOCK]; LANES];
        for ((end, body), tail) in ends.iter_mut().zip(bodies).zip(tails) {
            let from = &body[whole * BLOCK..];
            end[..from.len()].copy_from_slice(from);
            end[from.len()..rest].copy_from_slice(tail);
            end[rest] = 0x80;
            end[last - 16..last].copy_from_slice(&bits);
        }
        for at in (0..last).step_by(BLOCK) {
            compress(&mut state, std::array::from_fn(|l| block_at(&ends[l], at)));
        }
        let inner = unsplat(&state);

        // 对内部哈希做外部哈希：只有一个块
        let mut blocks = [[0u8; BLOCK]; LANES];
        for (block, inner) in blocks.iter_mut().zip(&inner) {
            super::put_words(block, inner);
            block[OUT] = 0x80;
            block[BLOCK - 16..].copy_from_slice(&(((BLOCK + OUT) * 8) as u128).to_be_bytes());
        }
        let mut state = splat(&key.outer);
        compress(&mut state, std::array::from_fn(|l| &blocks[l]));
        unsplat(&state).map(|words| {
            let mut tag = [0u8; OUT];
            super::put_words(&mut tag, &words);
            tag
        })
    }

    /// `m` 中从 `at` 开始的 128 字节。
    fn block_at(m: &[u8], at: usize) -> &[u8; BLOCK] {
        m[at..]
            .first_chunk::<BLOCK>()
            .unwrap_or_else(|| unreachable!("a whole block of the message"))
    }

    /// 把同一个状态放进全部四个通道。
    #[target_feature(enable = "avx2")]
    fn splat(words: &[u64; 8]) -> State {
        words.map(|w| _mm256_set1_epi64x(w.cast_signed()))
    }

    /// 取出四个通道各自的状态。
    #[target_feature(enable = "avx2")]
    fn unsplat(state: &State) -> [[u64; 8]; LANES] {
        let mut lanes = [[0u64; 8]; LANES];
        for (i, word) in state.iter().enumerate() {
            let mut four = [0u64; LANES];
            // SAFETY: `four` 正好容纳写入的 32 字节
            unsafe { _mm256_storeu_si256(four.as_mut_ptr().cast(), *word) };
            for (lane, w) in lanes.iter_mut().zip(four) {
                lane[i] = w;
            }
        }
        lanes
    }

    /// 把四个 64 位通道各循环右移 `$n` 位。
    macro_rules! rotr {
        ($x:expr, $n:literal) => {
            _mm256_or_si256(
                _mm256_srli_epi64::<$n>($x),
                _mm256_slli_epi64::<{ 64 - $n }>($x),
            )
        };
    }

    /// SHA-512 压缩：把 `blocks[l]` 压缩进 `state` 的第 `l` 个通道。
    #[target_feature(enable = "avx2")]
    fn compress(state: &mut State, blocks: [&[u8; BLOCK]; LANES]) {
        // 把每个 64 位通道中的大端字换成 CPU 的字节序
        let swap = _mm256_setr_epi8(
            7, 6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8, //
            7, 6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8,
        );
        // 消息字：四个块的第 `i` 个字放在 `w[i]` 中；每次从每个块读入四个字，再做转置
        let mut w = [_mm256_setzero_si256(); 16];
        for g in 0..4 {
            // SAFETY: 读取的是 128 字节块中的 `32 * g .. 32 * g + 32` 字节
            let r = blocks.map(|b| unsafe { _mm256_loadu_si256(b.as_ptr().add(32 * g).cast()) });
            let t0 = _mm256_unpacklo_epi64(r[0], r[1]);
            let t1 = _mm256_unpackhi_epi64(r[0], r[1]);
            let t2 = _mm256_unpacklo_epi64(r[2], r[3]);
            let t3 = _mm256_unpackhi_epi64(r[2], r[3]);
            w[4 * g] = _mm256_shuffle_epi8(_mm256_permute2x128_si256::<0x20>(t0, t2), swap);
            w[4 * g + 1] = _mm256_shuffle_epi8(_mm256_permute2x128_si256::<0x20>(t1, t3), swap);
            w[4 * g + 2] = _mm256_shuffle_epi8(_mm256_permute2x128_si256::<0x31>(t0, t2), swap);
            w[4 * g + 3] = _mm256_shuffle_epi8(_mm256_permute2x128_si256::<0x31>(t1, t3), swap);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        macro_rules! round {
            ($a:ident, $b:ident, $c:ident, $d:ident, $e:ident, $f:ident, $g:ident, $h:ident, $i:expr) => {
                let i: usize = $i;
                if i >= 16 {
                    let w15 = w[(i - 15) % 16];
                    let w2 = w[(i - 2) % 16];
                    let s0 = _mm256_xor_si256(
                        _mm256_xor_si256(rotr!(w15, 1), rotr!(w15, 8)),
                        _mm256_srli_epi64::<7>(w15),
                    );
                    let s1 = _mm256_xor_si256(
                        _mm256_xor_si256(rotr!(w2, 19), rotr!(w2, 61)),
                        _mm256_srli_epi64::<6>(w2),
                    );
                    w[i % 16] = _mm256_add_epi64(
                        _mm256_add_epi64(w[i % 16], s0),
                        _mm256_add_epi64(w[(i - 7) % 16], s1),
                    );
                }
                let hkw = _mm256_add_epi64(
                    $h,
                    _mm256_add_epi64(_mm256_set1_epi64x(K[i].cast_signed()), w[i % 16]),
                );
                let s1 = _mm256_xor_si256(
                    _mm256_xor_si256(rotr!($e, 14), rotr!($e, 18)),
                    rotr!($e, 41),
                );
                let ch = _mm256_xor_si256(_mm256_and_si256($e, $f), _mm256_andnot_si256($e, $g));
                let t1 = _mm256_add_epi64(_mm256_add_epi64(hkw, ch), s1);
                let s0 = _mm256_xor_si256(
                    _mm256_xor_si256(rotr!($a, 28), rotr!($a, 34)),
                    rotr!($a, 39),
                );
                let maj = _mm256_xor_si256(
                    _mm256_and_si256(_mm256_xor_si256($a, $b), _mm256_xor_si256($b, $c)),
                    $b,
                );
                $d = _mm256_add_epi64($d, t1);
                $h = _mm256_add_epi64(t1, _mm256_add_epi64(s0, maj));
            };
        }
        macro_rules! eight {
            ($i:expr) => {
                round!(a, b, c, d, e, f, g, h, $i);
                round!(h, a, b, c, d, e, f, g, $i + 1);
                round!(g, h, a, b, c, d, e, f, $i + 2);
                round!(f, g, h, a, b, c, d, e, $i + 3);
                round!(e, f, g, h, a, b, c, d, $i + 4);
                round!(d, e, f, g, h, a, b, c, $i + 5);
                round!(c, d, e, f, g, h, a, b, $i + 6);
                round!(b, c, d, e, f, g, h, a, $i + 7);
            };
        }
        eight!(0);
        eight!(8);
        eight!(16);
        eight!(24);
        eight!(32);
        eight!(40);
        eight!(48);
        eight!(56);
        eight!(64);
        eight!(72);

        for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = _mm256_add_epi64(*s, v);
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "测试数据的字节和移位有意回绕"
)]
mod tests {
    use super::*;
    use hmac::{Hmac, KeyInit, Mac};
    use proptest::prelude::*;

    fn reference(key: &[u8], body: &[u8], tail: &[u8]) -> [u8; OUT] {
        let mut mac = <Hmac<Sha512> as KeyInit>::new_from_slice(key).unwrap();
        mac.update(body);
        mac.update(tail);
        mac.finalize().into_bytes().into()
    }

    #[test]
    fn the_keyed_states_are_hmacs_ipad_and_opad_blocks() {
        for key in [&b""[..], &[7u8; 32], &[9u8; 128], &[1u8; 129]] {
            let keyed = HmacKey::new(key);
            // 空消息的 HMAC：内部哈希只处理密钥块，再做外部哈希
            let mut inner = keyed.inner;
            let mut block = [0u8; BLOCK];
            block[0] = 0x80;
            block[BLOCK - 16..].copy_from_slice(&((BLOCK * 8) as u128).to_be_bytes());
            compress512(&mut inner, &[block]);
            let mut outer_block = [0u8; BLOCK];
            put_words(&mut outer_block, &inner);
            outer_block[OUT] = 0x80;
            outer_block[BLOCK - 16..].copy_from_slice(&(((BLOCK + OUT) * 8) as u128).to_be_bytes());
            let mut tag = keyed.outer;
            compress512(&mut tag, &[outer_block]);
            let mut bytes = [0u8; OUT];
            put_words(&mut bytes, &tag);
            assert_eq!(
                bytes,
                reference(key, b"", b""),
                "key of {} bytes",
                key.len()
            );
        }
    }

    #[test]
    fn four_pages_hash_as_one_by_one() {
        let key = [0x5au8; 32];
        let pages: Vec<Vec<u8>> = (0..4u8)
            .map(|p| {
                (0..4032)
                    .map(|i| u8::try_from(i % 256).unwrap().wrapping_mul(p + 3))
                    .collect()
            })
            .collect();
        let numbers: Vec<[u8; 4]> = (2..6u32).map(u32::to_le_bytes).collect();
        let bodies = std::array::from_fn(|l| pages[l].as_slice());
        let tails = std::array::from_fn(|l| &numbers[l][..]);
        let tags = hmac4(&HmacKey::new(&key), bodies, tails);
        #[cfg(target_arch = "x86_64")]
        assert_eq!(
            tags.is_some(),
            std::arch::is_x86_feature_detected!("avx2"),
            "hashed side by side exactly where the CPU has AVX2"
        );
        let Some(tags) = tags else {
            return; // 由调用方逐条计算：没有可比较的
        };
        for l in 0..LANES {
            assert_eq!(tags[l], reference(&key, bodies[l], tails[l]), "lane {l}");
        }
    }

    #[test]
    fn pages_checked_together_pass_and_fail_as_one_by_one() {
        use crate::sqlcipher::testutil::{encrypt_page, KEY, SALT};
        use crate::sqlcipher::{PageCipher, PAGE_SIZE};
        let cipher = PageCipher::derive(&KEY, &SALT);
        let pages: Vec<(u32, Vec<u8>)> = (2..11u32)
            .map(|pgno| {
                let plain: Vec<u8> = (0..PAGE_SIZE).map(|i| (i as u32 ^ pgno) as u8).collect();
                (pgno, encrypt_page(&cipher, pgno, &plain, [pgno as u8; 16]))
            })
            .collect();
        let all: Vec<(u32, &[u8])> = pages.iter().map(|(p, b)| (*p, b.as_slice())).collect();
        // 每一种页数：四页一组，以及剩下的一、二、三页（剩两三页时按四页计算）
        for n in 0..=all.len() {
            let refs = &all[..n];
            cipher.verify_each(refs).unwrap();
            // 依次损坏每个位置上的页
            for bad in 0..n {
                let mut damaged = pages[bad].1.clone();
                damaged[100] ^= 1;
                let mut with = refs.to_vec();
                with[bad].1 = &damaged;
                let err = cipher.verify_each(&with).unwrap_err().to_string();
                assert!(
                    err.contains(&format!("page {}", refs[bad].0)),
                    "{n} pages, damaged page {bad}: {err}"
                );
                assert_eq!(
                    err,
                    cipher
                        .verify(refs[bad].0, &damaged)
                        .unwrap_err()
                        .to_string()
                );
            }
        }
        let refs = all;
        // 按另一个页号校验的页同样失败
        let mut swapped = refs.clone();
        swapped[1].0 = 99;
        assert!(cipher.verify_each(&swapped).is_err());
    }

    proptest! {
        #![proptest_config(crate::property_cases(64))]

        /// 任意的密钥、任意四个长度相同的消息主体（包括块边界附近的长度），以及长度相同的尾部。
        #[test]
        fn any_four_messages_hash_as_one_by_one(
            key in proptest::collection::vec(any::<u8>(), 0..200),
            len in 0usize..700,
            tail_len in 0usize..40,
            seed in any::<u64>(),
        ) {
            let byte = |l: usize, i: usize| {
                (seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(l as u32 * 7) >> (i % 56)) as u8 ^ (i as u8)
            };
            let bodies: Vec<Vec<u8>> = (0..LANES).map(|l| (0..len).map(|i| byte(l, i)).collect()).collect();
            let tails: Vec<Vec<u8>> = (0..LANES).map(|l| (0..tail_len).map(|i| byte(l + 4, i)).collect()).collect();
            let b = std::array::from_fn(|l| bodies[l].as_slice());
            let t = std::array::from_fn(|l| tails[l].as_slice());
            if let Some(tags) = hmac4(&HmacKey::new(&key), b, t) {
                for l in 0..LANES {
                    prop_assert_eq!(tags[l], reference(&key, b[l], t[l]), "lane {}", l);
                }
            }
        }
    }
}
