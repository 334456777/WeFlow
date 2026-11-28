//! 只输出一个块的 PBKDF2-HMAC-SHA512（RFC 8018）：用于派生数据库的页密钥，这是打开数据库时慢的那一步（见
//! [`crate::sqlcipher`]）。
//!
//! 256,000 轮中的每一轮都是对上一轮 64 字节的 HMAC。用通用的 HMAC 实现时，每一轮要复制一次已写入密钥的状态，
//! 做一次带缓冲的更新、一次填充和两次收尾。这里每一轮的输入形状相同：64 字节加上固定的填充，正好填满一个
//! 128 字节的块，所以填充只写一次，每一轮就是两次 SHA-512 压缩函数的调用，
//! 分别作用于只写入一次密钥的内部状态和外部状态（RFC 2104）。
//! 压缩函数用 `sha2` 的，它在运行时为 CPU 选出最快的实现。

use hmac::{Hmac, KeyInit, Mac};
use sha2::block_api::compress512;
use sha2::Sha512;

use crate::sha512x4::{put_words, words, HmacKey, BLOCK, OUT};

/// 按 RFC 8018 的定义，以 `password` 和 `salt` 做 `rounds` 轮 PBKDF2-HMAC-SHA512，取结果的前 `N` 字节（最多
/// 64 字节）；`rounds` 为 0 时按 1 轮算，与 `pbkdf2` crate 一致。
pub(crate) fn pbkdf2_hmac_sha512<const N: usize>(
    password: &[u8],
    salt: &[u8],
    rounds: u32,
) -> [u8; N] {
    const {
        assert!(
            N <= OUT,
            "one block of PBKDF2-HMAC-SHA512 gives at most 64 bytes"
        );
    };
    let key = HmacKey::new(password);

    // U1 = HMAC(password, salt || INT(1))：只算一次，用通用的 HMAC，因为盐可以是任意长度
    #[expect(clippy::expect_used, reason = "HMAC 接受任意长度的密钥")]
    let mut mac = <Hmac<Sha512> as KeyInit>::new_from_slice(password).expect("any key length");
    mac.update(salt);
    mac.update(&1u32.to_be_bytes());
    let u = words(&mac.finalize().into_bytes().into());
    let t = more_rounds(&key, u, rounds.saturating_sub(1));

    let mut bytes = [0u8; OUT];
    put_words(&mut bytes, &t);
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes[..N]);
    out
}

/// PBKDF2 的 U1 ^ U2 ^ ... ^ U(n+1)，已知 U1（`u`）：算第一轮之后的 `n` 轮，每一轮都是用 `key` 做的 HMAC。
fn more_rounds(key: &HmacKey, mut u: [u64; 8], n: u32) -> [u64; 8] {
    let mut t = u;
    // 前 64 字节放上一轮的结果，之后是长度为一个块（密钥的那一块）加 64 字节的消息的填充
    let mut block = [0u8; BLOCK];
    block[OUT] = 0x80;
    block[BLOCK - 16..].copy_from_slice(&(((BLOCK + OUT) * 8) as u128).to_be_bytes());
    for _ in 0..n {
        put_words(&mut block, &u);
        let mut h = key.inner;
        compress512(&mut h, &[block]);
        put_words(&mut block, &h);
        u = key.outer;
        compress512(&mut u, &[block]);
        for (t, u) in t.iter_mut().zip(&u) {
            *t ^= u;
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn reference(password: &[u8], salt: &[u8], rounds: u32) -> [u8; OUT] {
        let mut out = [0u8; OUT];
        pbkdf2::pbkdf2_hmac::<Sha512>(password, salt, rounds, &mut out);
        out
    }

    #[test]
    fn a_database_key_derives_as_the_pbkdf2_crate_derives_it() {
        use crate::sqlcipher::testutil::{KEY, SALT};
        let ours: [u8; 32] = pbkdf2_hmac_sha512(&KEY, &SALT, crate::sqlcipher::KDF_ITER);
        assert_eq!(
            ours[..],
            reference(&KEY, &SALT, crate::sqlcipher::KDF_ITER)[..32]
        );
    }

    #[test]
    fn the_rfc_test_vector_of_one_round_comes_out() {
        // `PBKDF2-HMAC-SHA512("password", "salt", 1, 64)`，即 `pbkdf2` crate 测试用的向量
        let ours: [u8; 64] = pbkdf2_hmac_sha512(b"password", b"salt", 1);
        assert_eq!(ours, reference(b"password", b"salt", 1));
        assert_eq!(
            ours[..8],
            [0x86, 0x7f, 0x70, 0xcf, 0x1a, 0xde, 0x02, 0xcf],
            "{ours:02x?}"
        );
    }

    proptest! {
        #![proptest_config(crate::property_cases(64))]

        /// 任意的口令（包括比一个块长的）、盐、轮数，以及不超过一个块的输出长度。
        #[test]
        fn any_input_derives_what_the_pbkdf2_crate_derives(
            password in proptest::collection::vec(any::<u8>(), 0..300),
            salt in proptest::collection::vec(any::<u8>(), 0..300),
            rounds in 0u32..40,
        ) {
            let want = reference(&password, &salt, rounds);
            let full: [u8; 64] = pbkdf2_hmac_sha512(&password, &salt, rounds);
            prop_assert_eq!(full, want);
            let short: [u8; 32] = pbkdf2_hmac_sha512(&password, &salt, rounds);
            prop_assert_eq!(&short[..], &want[..32]);
        }
    }
}
