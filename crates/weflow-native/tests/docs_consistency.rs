//! Keeps the Markdown docs self-consistent: relative links and `#anchors` resolve, every English doc has a
//! zh-CN twin (and the reverse), and every crate is listed in the architecture tables of `native-cli.md`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Docs covered by the checks. `weflow-tech-docs/` and `es-ES/` are outside the bilingual rule in CLAUDE.md.
fn doc_files() -> Vec<PathBuf> {
    let root = root();
    let mut files = vec![root.join("README.md")];
    for dir in ["docs", "docs/zh-CN"] {
        for entry in std::fs::read_dir(root.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "md") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn rel(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Text with fenced code blocks and inline code removed, so examples such as `[title](URL)` are not links.
fn strip_code(text: &str) -> String {
    let inline = Regex::new(r"`[^`\n]*`").unwrap();
    let mut out = String::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            out.push_str(&inline.replace_all(line, ""));
            out.push('\n');
        }
    }
    out
}

/// GitHub's heading slug: lowercase, drop punctuation except `-` and `_`, spaces become `-`.
fn slug(heading: &str) -> String {
    let plain = Regex::new(r"\[([^\]]*)\]\([^)]*\)")
        .unwrap()
        .replace_all(heading, "$1")
        .replace(['`', '*'], "");
    plain
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

fn anchors(text: &str) -> HashSet<String> {
    let heading = Regex::new(r"^#{1,6}\s+(.*)$").unwrap();
    let html_id = Regex::new(r#"id="([^"]+)""#).unwrap();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = HashSet::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some(c) = heading.captures(line) {
            let base = slug(&c[1]);
            let n = seen.entry(base.clone()).or_insert(0);
            out.insert(if *n == 0 { base } else { format!("{base}-{n}") });
            *n += 1;
        }
        for c in html_id.captures_iter(line) {
            out.insert(c[1].to_string());
        }
    }
    out
}

#[test]
fn relative_links_and_anchors_resolve() {
    let link = Regex::new(r"\]\(([^)\s]+)\)").unwrap();
    let mut problems = Vec::new();
    for file in doc_files() {
        let text = std::fs::read_to_string(&file).unwrap();
        for c in link.captures_iter(&strip_code(&text)) {
            let url = &c[1];
            if url.starts_with("http://")
                || url.starts_with("https://")
                || url.starts_with("mailto:")
            {
                continue;
            }
            let (path, anchor) = url.split_once('#').unwrap_or((url, ""));
            let target = if path.is_empty() {
                file.clone()
            } else {
                file.parent().unwrap().join(path)
            };
            if !target.exists() {
                problems.push(format!("{}: missing file {url}", rel(&file)));
            } else if !anchor.is_empty() && target.extension().is_some_and(|e| e == "md") {
                let found = anchors(&std::fs::read_to_string(&target).unwrap());
                if !found.contains(anchor) {
                    problems.push(format!("{}: no anchor in {url}", rel(&file)));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "broken doc links:\n{}",
        problems.join("\n")
    );
}

#[test]
fn every_doc_has_a_twin_in_the_other_language() {
    let docs = root().join("docs");
    let md_names = |dir: PathBuf| -> HashSet<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".md"))
            .collect()
    };
    // The English root README lives at the repository root, not in docs/.
    let mut english = md_names(docs.clone());
    english.insert("README.md".to_string());
    let chinese = md_names(docs.join("zh-CN"));
    let mut only_en: Vec<_> = english.difference(&chinese).collect();
    let mut only_zh: Vec<_> = chinese.difference(&english).collect();
    only_en.sort();
    only_zh.sort();
    assert!(
        only_en.is_empty() && only_zh.is_empty(),
        "docs/ and docs/zh-CN/ differ: only in English {only_en:?}, only in Chinese {only_zh:?}"
    );
}

#[test]
fn every_crate_is_in_the_architecture_tables() {
    let crates: Vec<String> = std::fs::read_dir(root().join("crates"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("Cargo.toml").exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        crates.len() >= 6,
        "found crates {crates:?}; did the layout change?"
    );
    for doc in ["docs/native-cli.md", "docs/zh-CN/native-cli.md"] {
        let text = std::fs::read_to_string(root().join(doc)).unwrap();
        let missing: Vec<&String> = crates
            .iter()
            .filter(|c| !text.contains(&format!("| `crates/{c}` |")))
            .collect();
        assert!(
            missing.is_empty(),
            "{doc} architecture table lacks: {missing:?}"
        );
    }
}
