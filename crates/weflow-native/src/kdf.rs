//! PBKDF2-HMAC-SHA512 (RFC 8018) for one output block: the derivation of a database's page key, the slow step of
//! opening it (see [`crate::sqlcipher`]).
//!
//! Each of its 256,000 rounds is an HMAC of the previous round's 64 bytes. Through a general HMAC that is a copy of
//! the keyed state, a buffered update, padding and two finalizations a round. Here every round's input has the same
//! shape, 64 bytes and fixed padding that fill one 128-byte block, so the padding is written once and a round is two
//! calls of the SHA-512 compression function, on the inner and outer states keyed once (RFC 2104). The compression
//! function is `sha2`'s, which picks the fastest one for the CPU at run time.

use hmac::{Hmac, KeyInit, Mac};
use sha2::block_api::compress512;
use sha2::Sha512;

use crate::sha512x4::{put_words, words, HmacKey, BLOCK, OUT};

/// The first `N` bytes (at most 64) of PBKDF2-HMAC-SHA512 of `password` and `salt` with `rounds` rounds, as RFC 8018
/// defines it (`rounds` 0 counts as 1, as the `pbkdf2` crate does).
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

    // U1 = HMAC(password, salt || INT(1)): once, through the general HMAC, as the salt has any length
    #[expect(clippy::expect_used, reason = "HMAC accepts a key of any length")]
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

/// U1 ^ U2 ^ ... ^ U(n+1) of PBKDF2, given U1 (`u`): the `n` rounds after the first, each an HMAC with `key`.
fn more_rounds(key: &HmacKey, mut u: [u64; 8], n: u32) -> [u64; 8] {
    let mut t = u;
    // the 64 bytes of the previous round, then the padding of a message of one block (the key's) and 64 bytes
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
        // PBKDF2-HMAC-SHA512("password", "salt", 1, 64), the vector the `pbkdf2` crate tests with
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

        /// Any password (longer than a block too), salt, round count and output length up to a block.
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
