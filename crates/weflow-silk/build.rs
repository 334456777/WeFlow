use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("vendor");
    let src = root.join("src");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&src)
        .expect("vendored SILK sources")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "c"))
        .collect();
    files.sort();
    let mut build = cc::Build::new();
    build
        .include(&src)
        .include(root.join("interface"))
        .files(&files)
        .warnings(false)
        .opt_level(2);
    build.compile("silk");
    println!("cargo:rerun-if-changed=vendor");
}
