//! Keeps the Markdown in the repository self-consistent: the README's relative links and `#anchors` resolve, and
//! commas next to Chinese characters are full-width (`，`). The commas are only reported, never rewritten.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Docs covered by the checks. The documentation itself is in the GitHub Wiki, outside this repository.
fn doc_files() -> Vec<PathBuf> {
    vec![root().join("README.md")]
}

fn rel(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Text with fenced code blocks blanked and inline code replaced by a placeholder, so examples such as
/// `[title](URL)` are not links. Line numbers are preserved.
fn strip_code(text: &str) -> String {
    let inline = Regex::new(r"`[^`\n]*`").unwrap();
    let mut out = String::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            out.push_str(&inline.replace_all(line, "\u{1}"));
        }
        out.push('\n');
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

/// An ASCII comma next to a Han character, optionally with closing/opening brackets, emphasis marks or spaces in
/// between (`(可选绑定), 也`).
#[test]
fn commas_next_to_chinese_are_full_width() {
    let han = r"[\u{4e00}-\u{9fff}]";
    let after_han = Regex::new(&format!(r"{han}[)\]」”’*_ ]*,")).unwrap();
    let before_han = Regex::new(&format!(r",[(\[「“‘*_ ]*{han}")).unwrap();
    let mut files = doc_files();
    files.push(root().join("CLAUDE.md"));
    let mut problems = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for (i, line) in strip_code(&text).lines().enumerate() {
            for m in after_han.find_iter(line).chain(before_han.find_iter(line)) {
                problems.push(format!("{}:{}: \"{}\"", rel(&file), i + 1, m.as_str()));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "use the full-width comma ， next to Chinese text (not fixed automatically):\n{}",
        problems.join("\n")
    );
}
