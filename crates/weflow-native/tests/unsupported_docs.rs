//! Keeps `docs/cli-unsupported.md` (and its zh-CN twin) in sync with the code: every database function that
//! answers "not implemented" or "not supported" must be listed there.

use std::path::Path;

use regex::Regex;

/// Names of the `Wcdb` methods whose whole body is `self.pending("..")` or `self.read_only("..")`.
fn refusing_functions(source: &str) -> Vec<String> {
    let re = Regex::new(r#"pub fn (\w+)\([^{]*\{\s*self\.(?:pending|read_only)\(""#).unwrap();
    re.captures_iter(source).map(|c| c[1].to_string()).collect()
}

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn every_refusing_function_is_documented_in_both_languages() {
    let names = refusing_functions(&read("src/wcdb.rs"));
    assert!(
        names.len() >= 12,
        "the source scan found only {names:?}; did the stub layout change?"
    );
    for doc in [
        "../../docs/cli-unsupported.md",
        "../../docs/zh-CN/cli-unsupported.md",
    ] {
        let text = read(doc);
        let missing: Vec<&String> = names
            .iter()
            .filter(|n| !text.contains(&format!("`{n}`")))
            .collect();
        assert!(missing.is_empty(), "{doc} does not list: {missing:?}");
    }
}

#[test]
fn documented_functions_still_exist_and_still_refuse() {
    // a row that stays in the doc after its function was ported is just as wrong as a missing row
    let names = refusing_functions(&read("src/wcdb.rs"));
    let doc = read("../../docs/cli-unsupported.md");
    let section = doc.split("## 3.").next().expect("section 3 exists");
    let documented = Regex::new(r"`([a-z_0-9]+)`").unwrap();
    let source = read("src/wcdb.rs");
    for c in documented.captures_iter(section) {
        let n = &c[1];
        // only identifiers that look like native function names and exist as Wcdb methods
        if source.contains(&format!("pub fn {n}(")) {
            assert!(
                names.iter().any(|x| x == n),
                "`{n}` is listed as unsupported but is implemented now"
            );
        }
    }
}

#[test]
fn readmes_point_to_the_list() {
    assert!(read("../../README.md").contains("docs/cli-unsupported.md"));
    assert!(read("../../README.zh-CN.md").contains("docs/zh-CN/cli-unsupported.md"));
}
