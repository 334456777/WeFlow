//! Random numbers: the account's own generator and weighted draws.

/// SplitMix64: small, fast, and fixed for good, where a library's generator may change its stream in a new version.
#[derive(Clone)]
pub(super) struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// A stream of its own for one part of the account, so changing one part leaves the others as they were.
    pub(super) fn fork(&self, salt: u64) -> Self {
        let mut r = Self(self.0 ^ salt.wrapping_mul(0xA24B_AED4_963E_E407));
        r.next_u64();
        r
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub(super) fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `0..n` (0 when `n` is 0).
    pub(super) fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// Uniform in `lo..hi`.
    pub(super) fn between(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % (hi - lo).max(1) as u64) as i64
    }

    pub(super) fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    pub(super) fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// A standard normal draw (Box–Muller).
    fn normal(&mut self) -> f64 {
        let u = 1.0 - self.unit();
        let v = self.unit();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }

    /// A log-normal draw with this median and 90th percentile, at most `max`.
    pub(super) fn spread(&mut self, (median, p90, max): (f64, f64, f64)) -> f64 {
        let sigma = (p90 / median).ln() / 1.2816;
        (median * (sigma * self.normal()).exp()).min(max)
    }

    pub(super) fn hex(&mut self, n: usize) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(n);
        while out.len() < n {
            let mut bits = self.next_u64();
            for _ in 0..16.min(n - out.len()) {
                out.push(DIGITS[(bits & 15) as usize] as char);
                bits >>= 4;
            }
        }
        out
    }

    /// `n` hex digits as XML carries them in URLs and keys: random runs, some of them repeated (the same key or
    /// parameter twice), which compresses about as well as WeChat's does.
    pub(super) fn filler(&mut self, n: usize) -> String {
        let mut out = String::with_capacity(n);
        while out.len() < n {
            let run = (16 + self.below(48)).min(n - out.len());
            if out.len() >= 64 && self.chance(0.45) {
                let from = self.below(out.len() - run.min(out.len()));
                let copy = out[from..from + run.min(out.len() - from)].to_string();
                out.push_str(&copy);
            } else {
                let hex = self.hex(run);
                out.push_str(&hex);
            }
        }
        out.truncate(n);
        out
    }

    pub(super) fn bytes(&mut self, n: usize) -> Vec<u8> {
        let mut out = vec![0u8; n];
        for chunk in out.chunks_mut(8) {
            let b = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&b[..chunk.len()]);
        }
        out
    }
}

/// Draws an index with the weights it was made with.
pub(super) struct Weighted(Vec<f64>);

impl Weighted {
    pub(super) fn new(weights: impl IntoIterator<Item = f64>) -> Self {
        let mut total = 0.0;
        Self(
            weights
                .into_iter()
                .map(|w| {
                    total += w;
                    total
                })
                .collect(),
        )
    }

    pub(super) fn draw(&self, rng: &mut Rng) -> usize {
        let x = rng.unit() * self.0.last().copied().unwrap_or(0.0);
        self.0.partition_point(|&c| c <= x).min(self.0.len() - 1)
    }
}

/// Draws a value from `(from, to, share)` ranges: a range by its share, then in it log-uniformly (short values are
/// more common than long ones inside a range too).
pub(super) fn draw_range<T: Copy + Into<f64>>(rng: &mut Rng, ranges: &[(T, T, f64)]) -> f64 {
    let (from, to, _) = ranges[Weighted::new(ranges.iter().map(|r| r.2)).draw(rng)];
    let (from, to): (f64, f64) = (from.into().max(1.0), to.into());
    (from.ln() + rng.unit() * (to.ln() - from.ln())).exp()
}
