//! Inspect filesystem access without printing path names or file contents.
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
