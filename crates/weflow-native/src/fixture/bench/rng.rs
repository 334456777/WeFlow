//! 随机数：基准账号自己的生成器和按权重抽取。

/// [`Rng::filler`] 中连续十六进制数字的最短和最长长度：
/// 真实账号中 32 位及以上的连续数字除少数外都短于 256 位（#1194）。
const FILLER_RUNS: (i64, i64) = (32, 200);
/// 截断 [`Rng::filler`] 中连续数字的分隔符，与微信的 CDN 地址相同；
/// 它们在 XML 中都不需要转义，所以填充在任何位置截断都仍然合法。
const FILLER_SEPARATORS: &[&str] = &["/", "_", "-", "?m=", "=", "."];

/// SplitMix64：小、快，而且永远不变；库里的生成器在新版本中可能改变输出的序列。
#[derive(Clone)]
pub(super) struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// 给账号的一部分用的独立序列，改动一部分时其他部分保持原样。
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

    /// `[0, 1)` 上的均匀分布。
    pub(super) fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// `0..n` 上的均匀分布（`n` 为 0 时为 0）。
    pub(super) fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// `lo..hi` 上的均匀分布。
    pub(super) fn between(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % (hi - lo).max(1) as u64) as i64
    }

    pub(super) fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    pub(super) fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// 标准正态分布的一次抽样（Box–Muller 变换）。
    fn normal(&mut self) -> f64 {
        let u = 1.0 - self.unit();
        let v = self.unit();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }

    /// 中位数和第 90 百分位数为给定值的对数正态分布的一次抽样，不超过 `max`。
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

    /// XML 的地址和密钥中带的 `n` 字节十六进制内容：随机的片段，其中一些重复（同一个密钥或参数出现两次），
    /// 压缩率与微信的大致相同；每隔 [`FILLER_RUNS`] 个数字用地址的分隔符截断，
    /// 因为真实的消息把长度分散在几个密钥和参数里（#1194）。
    pub(super) fn filler(&mut self, n: usize) -> String {
        let mut out = String::with_capacity(n);
        let mut digits = 0;
        let mut run_max = self.between(FILLER_RUNS.0, FILLER_RUNS.1 + 1);
        while out.len() < n {
            let run = (16 + self.below(48)).min(n - out.len());
            let piece = if out.len() >= 64 && self.chance(0.45) {
                let from = self.below(out.len() - run.min(out.len()));
                out[from..from + run.min(out.len() - from)].to_string()
            } else {
                self.hex(run)
            };
            for c in piece.chars() {
                if !c.is_ascii_hexdigit() {
                    // 随前面的片段一起复制过来的分隔符
                    digits = 0;
                } else if digits == run_max {
                    out.push_str(self.pick(FILLER_SEPARATORS));
                    digits = 0;
                    run_max = self.between(FILLER_RUNS.0, FILLER_RUNS.1 + 1);
                }
                if c.is_ascii_hexdigit() {
                    digits += 1;
                }
                out.push(c);
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

/// 按构造时给的权重抽取一个序号。
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

/// 从 `(from, to, share)` 的几个范围中抽一个值：先按比例选一个范围，再在其中按对数均匀分布抽取，
/// 所以范围内短的值也比长的值常见。
pub(super) fn draw_range<T: Copy + Into<f64>>(rng: &mut Rng, ranges: &[(T, T, f64)]) -> f64 {
    let (from, to, _) = ranges[Weighted::new(ranges.iter().map(|r| r.2)).draw(rng)];
    let (from, to): (f64, f64) = (from.into().max(1.0), to.into());
    (from.ln() + rng.unit() * (to.ln() - from.ln())).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filler_has_the_length_asked_and_no_run_of_hex_longer_than_a_url_holds() {
        let mut rng = Rng::new(7);
        for n in [0, 1, 31, 200, 5_000, 50_000] {
            let fill = rng.filler(n);
            assert_eq!(fill.len(), n);
            let longest = fill
                .split(|c: char| !c.is_ascii_hexdigit())
                .map(str::len)
                .max()
                .unwrap_or(0);
            assert!(
                longest <= FILLER_RUNS.1 as usize,
                "a run of {longest} in {n}"
            );
            if n >= 5_000 {
                assert!(fill.chars().any(|c| !c.is_ascii_hexdigit()));
            }
        }
        assert_eq!(Rng::new(3).filler(4_000), Rng::new(3).filler(4_000));
    }
}
