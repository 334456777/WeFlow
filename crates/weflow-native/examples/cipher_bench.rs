//! 测量读取加密数据库的两项开销：派生一个数据库的页密钥（PBKDF2-HMAC-SHA512，256000 轮，每个数据库一次），
//! 以及解密一页（先校验 HMAC-SHA512，再 AES-256-CBC 解密）。用合成的页测量，不读任何账号。
//! 用 release 构建运行，比较加密 crate 的不同版本（#371）。
//! 用法：`cipher_bench [pages=20000] [rounds=5]`
#![expect(clippy::expect_used, reason = "探针用 panic 报告失败")]
#![expect(
    clippy::cast_possible_truncation,
    reason = "探针和基准的数据都是放得下的小值"
)]
use std::hint::black_box;
use std::time::Instant;

use weflow_native::sqlcipher::testutil::{encrypt_page, KEY, SALT};
use weflow_native::sqlcipher::{PageCipher, PAGE_SIZE};

fn main() {
    let mut args = std::env::args().skip(1);
    let pages: u32 = args.next().map_or(20_000, |a| a.parse().expect("pages"));
    let rounds: usize = args.next().map_or(5, |a| a.parse().expect("rounds"));

    let cipher = PageCipher::derive(&KEY, &SALT);
    let encrypted: Vec<Vec<u8>> = (2..pages + 2)
        .map(|pgno| {
            let plain: Vec<u8> = (0..PAGE_SIZE).map(|i| (i as u32 ^ pgno) as u8).collect();
            encrypt_page(&cipher, pgno, &plain, [pgno as u8; 16])
        })
        .collect();

    let mut derive = Vec::new();
    let mut decrypt = Vec::new();
    let mut out = vec![0u8; PAGE_SIZE];
    for _ in 0..rounds {
        let t = Instant::now();
        black_box(PageCipher::derive(black_box(&KEY), black_box(&SALT)));
        derive.push(t.elapsed().as_secs_f64() * 1000.0);

        let t = Instant::now();
        for (i, page) in encrypted.iter().enumerate() {
            cipher
                .decrypt_page(i as u32 + 2, page, &mut out)
                .expect("decrypt");
            black_box(&out);
        }
        decrypt.push(t.elapsed().as_secs_f64());
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    #[expect(clippy::cast_precision_loss, reason = "一页是 4096 字节")]
    let mb = f64::from(pages) * PAGE_SIZE as f64 / 1e6;
    let d = median(&mut decrypt);
    println!(
        "key derivation: {:.1} ms (median of {rounds})",
        median(&mut derive)
    );
    println!(
        "page decryption: {:.0} MB/s, {:.2} us/page ({pages} pages, median of {rounds})",
        mb / d,
        d * 1e6 / f64::from(pages)
    );
}
