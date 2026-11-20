//! Keeps the Markdown in the repository self-consistent: the relative links and `#anchors` of the READMEs and the skills
//! resolve, commas next to Chinese characters are full-width (`，`), and every skill has the front matter of the Agent
//! Skills format. The commas are only reported, never rewritten.
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

/// The skills: Agent Skills, which most agents read here and Claude Code through the `.claude/skills` symlink.
fn skills_dir() -> PathBuf {
    root().join(".agents/skills")
}

/// The Markdown files under `dir` and its subfolders, in path order.
fn markdown_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "md") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Docs covered by the checks: the READMEs and the skills. The documentation itself is in the GitHub Wiki, outside
/// this repository.
fn doc_files() -> Vec<PathBuf> {
    let mut files = vec![root().join("README.md"), root().join("README.zh-CN.md")];
    files.extend(markdown_under(&skills_dir()));
    files
}

/// The `key: value` lines of a Markdown file's front matter, between the `---` on its first line and the next `---`.
/// Empty when the file has none.
fn front_matter(text: &str) -> HashMap<String, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return HashMap::new();
    }
    lines
        .take_while(|line| line.trim_end() != "---")
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect()
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
/// between (`(可选绑定), 也`). Link targets are left out, so an English link to a Chinese Wiki anchor
/// (`[contacts](…#联系人与群), …`) is not a match, while a Chinese link text before a comma still is.
#[test]
fn commas_next_to_chinese_are_full_width() {
    let han = r"[\u{4e00}-\u{9fff}]";
    let after_han = Regex::new(&format!(r"{han}[)\]」”’*_ ]*,")).unwrap();
    let before_han = Regex::new(&format!(r",[(\[「“‘*_ ]*{han}")).unwrap();
    let link_target = Regex::new(r"\]\([^)\s]*\)").unwrap();
    let mut files = doc_files();
    files.push(root().join("CLAUDE.md"));
    let mut problems = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for (i, line) in strip_code(&text).lines().enumerate() {
            let line = link_target.replace_all(line, "]");
            for m in after_han
                .find_iter(&line)
                .chain(before_han.find_iter(&line))
            {
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

/// Every folder in `.agents/skills` is a skill in the Agent Skills format: its `SKILL.md` starts with front matter
/// whose `name` is the folder's name (lowercase letters and digits joined by single hyphens, at most 64 characters)
/// and whose `description`, which agents read to decide when to use the skill, is one line of at most 1024 characters.
#[test]
fn skills_follow_the_agent_skills_format() {
    let name_form = Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").unwrap();
    let mut problems = Vec::new();
    let mut skills = 0;
    for entry in std::fs::read_dir(skills_dir()).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        skills += 1;
        let folder = rel(&dir);
        let Ok(text) = std::fs::read_to_string(dir.join("SKILL.md")) else {
            problems.push(format!("{folder}: no SKILL.md"));
            continue;
        };
        let fields = front_matter(&text);
        let name = fields.get("name").map_or("", String::as_str);
        let expected = dir.file_name().unwrap().to_string_lossy();
        if name != expected {
            problems.push(format!(
                "{folder}: name is \"{name}\", not the folder's name"
            ));
        } else if !name_form.is_match(name) || name.len() > 64 {
            problems.push(format!(
                "{folder}: name must be lowercase letters and digits joined by single hyphens, at most 64 characters"
            ));
        }
        let description = fields.get("description").map_or("", String::as_str);
        if description.is_empty() || description.starts_with(['|', '>']) {
            problems.push(format!("{folder}: description must be one non-empty line"));
        } else if description.chars().count() > 1024 {
            problems.push(format!(
                "{folder}: description has {} characters, more than 1024",
                description.chars().count()
            ));
        }
    }
    assert!(skills > 0, "no skills found in {}", rel(&skills_dir()));
    assert!(
        problems.is_empty(),
        "skills not in the Agent Skills format:\n{}",
        problems.join("\n")
    );
}
