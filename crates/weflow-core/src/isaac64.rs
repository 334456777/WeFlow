//! ISAAC-64 keystream used to decrypt WeChat Moments images and videos.
//!
//! The desktop app ships the vendor's WASM (`WxIsaac64`) for this; its output is
//! the textbook ISAAC-64 generator seeded with the decimal key in `randrsl[0]`,
//! emitting each batch from `randrsl[255]` down to `randrsl[0]`, every 64-bit word
//! serialised big-endian. (The pure-TypeScript fallback in `electron/services/isaac64.ts`
//! indexes `mm` through a lossy `Number()` conversion and does not produce this stream;
//! the vectors in the tests were captured from the real WASM module.)

const SIZE: usize = 256;

pub struct Isaac64 {
    mm: [u64; SIZE],
    rs: [u64; SIZE],
    aa: u64,
    bb: u64,
    cc: u64,
    count: usize,
}

impl Isaac64 {
    pub fn new(seed: u64) -> Self {
        let mut s = Isaac64 {
            mm: [0; SIZE],
            rs: [0; SIZE],
            aa: 0,
            bb: 0,
            cc: 0,
            count: 0,
        };
        s.rs[0] = seed;
        // golden ratio after four mix rounds (precomputed in the reference implementation)
        let mut v: [u64; 8] = [
            0x647c4677a2884b7c,
            0xb9f8b322c73ac862,
            0x8c0ea5053d4712a0,
            0xb29b2e824a595524,
            0x82f053db8355e0ce,
            0x48fe4a0fa5a09315,
            0xae985bf2cbfc89ed,
            0x98f5704f6c44c0ab,
        ];
        for pass in 0..2 {
            for i in (0..SIZE).step_by(8) {
                for j in 0..8 {
                    let add = if pass == 0 { s.rs[i + j] } else { s.mm[i + j] };
                    v[j] = v[j].wrapping_add(add);
                }
                mix(&mut v);
                s.mm[i..i + 8].copy_from_slice(&v);
            }
        }
        s.refill();
        s
    }

    /// Parses the decimal key the way `strtoull(key, NULL, 10)` does (leading digits only).
    pub fn from_key(key: &str) -> Self {
        let digits: String = key
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        Self::new(digits.parse::<u64>().unwrap_or(0))
    }

    fn refill(&mut self) {
        self.cc = self.cc.wrapping_add(1);
        self.bb = self.bb.wrapping_add(self.cc);
        for i in 0..SIZE {
            let x = self.mm[i];
            self.aa = match i & 3 {
                0 => self.aa ^ !(self.aa << 21),
                1 => self.aa ^ (self.aa >> 5),
                2 => self.aa ^ (self.aa << 12),
                _ => self.aa ^ (self.aa >> 33),
            };
            self.aa = self.mm[(i + SIZE / 2) & (SIZE - 1)].wrapping_add(self.aa);
            let y = self.mm[((x >> 3) & 255) as usize]
                .wrapping_add(self.aa)
                .wrapping_add(self.bb);
            self.mm[i] = y;
            self.bb = self.mm[((y >> 11) & 255) as usize].wrapping_add(x);
            self.rs[i] = self.bb;
        }
        self.count = SIZE;
    }

    pub fn next_u64(&mut self) -> u64 {
        if self.count == 0 {
            self.refill();
        }
        self.count -= 1;
        self.rs[self.count]
    }

    /// `size` bytes of keystream (big-endian words, truncated to `size`).
    pub fn keystream(&mut self, size: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(size + 8);
        while out.len() < size {
            out.extend_from_slice(&self.next_u64().to_be_bytes());
        }
        out.truncate(size);
        out
    }
}

fn mix(v: &mut [u64; 8]) {
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *v;
    a = a.wrapping_sub(e);
    f ^= h >> 9;
    h = h.wrapping_add(a);
    b = b.wrapping_sub(f);
    g ^= a << 9;
    a = a.wrapping_add(b);
    c = c.wrapping_sub(g);
    h ^= b >> 23;
    b = b.wrapping_add(c);
    d = d.wrapping_sub(h);
    a ^= c << 15;
    c = c.wrapping_add(d);
    e = e.wrapping_sub(a);
    b ^= d >> 14;
    d = d.wrapping_add(e);
    f = f.wrapping_sub(b);
    c ^= e << 20;
    e = e.wrapping_add(f);
    g = g.wrapping_sub(c);
    d ^= f >> 17;
    f = f.wrapping_add(g);
    h = h.wrapping_sub(d);
    e ^= g << 14;
    g = g.wrapping_add(h);
    *v = [a, b, c, d, e, f, g, h];
}

/// XOR `data` with the keystream for `key`; with `limit`, only the first `limit` bytes
/// are touched (video headers only encrypt the first 128 KiB).
pub fn xor_in_place(data: &mut [u8], key: &str, limit: Option<usize>) {
    let n = limit.map_or(data.len(), |l| l.min(data.len()));
    let ks = Isaac64::from_key(key).keystream(n);
    for (b, k) in data[..n].iter_mut().zip(ks) {
        *b ^= k;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    // Vectors captured from the vendor WASM module (wasm_video_decode.wasm, WxIsaac64).
    #[test]
    fn matches_vendor_wasm_first_batch() {
        let ks = Isaac64::from_key("2105122989").keystream(32);
        assert_eq!(
            hex(&ks),
            "89cd4cd8f38b2190b5756244f6d7c75f9ea92584164b69b400b924e93acf563c"
        );
        assert_eq!(
            hex(&Isaac64::from_key("1").keystream(13)),
            "e19ed5d2ca98af2da7a18d07ca"
        );
    }

    #[test]
    fn matches_vendor_wasm_across_batches() {
        let ks = Isaac64::from_key("2105122989").keystream(4200);
        assert_eq!(
            hex(&ks[2040..2072]),
            "a1f2d5e4f943482900b4cc589cce41bc045143d889fad6b1172e530846d8a4e3"
        );
        assert_eq!(
            hex(&ks[4168..4200]),
            "20870d6e9921d2121cd4d13ed157ed55bf8b35498cb696fdcb19d6227f3a120c"
        );
    }

    #[test]
    fn xor_is_involutive_and_respects_limit() {
        let original: Vec<u8> = (0..300u32).map(|i| i as u8).collect();
        let mut data = original.clone();
        xor_in_place(&mut data, "42", Some(100));
        assert_ne!(data[..100], original[..100]);
        assert_eq!(data[100..], original[100..]);
        xor_in_place(&mut data, "42", Some(100));
        assert_eq!(data, original);
    }
}
