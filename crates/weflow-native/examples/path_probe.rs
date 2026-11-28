//! 检查账号目录、`db_storage` 和 `session.db` 能否访问：打印各自是否为目录、是否为文件和长度，或读取元数据的错误，
//! 再打印临时目录是否就是 `TMP`。不打印路径名和文件内容。
#![expect(clippy::expect_used, reason = "探针用 panic 报告失败")]
fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("path_probe <account>"));
    for (kind, path) in [
        ("account", root.clone()),
        ("db_storage", root.join("db_storage")),
        ("session", root.join("db_storage/session/session.db")),
    ] {
        match std::fs::metadata(&path) {
            Ok(m) => println!(
                "{kind}: dir={} file={} len={}",
                m.is_dir(),
                m.is_file(),
                m.len()
            ),
            Err(e) => println!("{kind}: metadata error {e:?}"),
        }
    }
    println!(
        "tempdir equals TMP: {}",
        std::env::var_os("TMP").is_some_and(|s| std::env::temp_dir() == s)
    );
}
