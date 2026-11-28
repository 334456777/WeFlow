//! 让仓库里的 Markdown 保持自洽：README、CLAUDE.md 和 skill 中的相对链接和锚点（`#…`）都能找到，
//! 汉字旁的逗号是全角的（`，`），每个 skill 都有 Agent Skills 格式的 front matter，
//! 智能体读取的符号链接检出为链接。逗号只报告，不自动改写。
#![expect(clippy::unwrap_used, reason = "测试辅助函数用 panic 让测试失败")]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// skill 所在的目录：Agent Skills，多数智能体从这里读取，Claude Code 经 `.claude/skills` 符号链接读取。
fn skills_dir() -> PathBuf {
    root().join(".agents/skills")
}

/// `dir` 及其子目录中的 Markdown 文件，按路径排序。
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

/// 检查覆盖的文档：两份 README、给智能体的说明（CLAUDE.md）和各个 skill。
/// 文档本身在 GitHub Wiki 里，不在这个仓库中。
fn doc_files() -> Vec<PathBuf> {
    let mut files = vec![
        root().join("README.md"),
        root().join("README.zh-CN.md"),
        root().join("CLAUDE.md"),
    ];
    files.extend(markdown_under(&skills_dir()));
    files
}

/// Markdown 文件 front matter 中的 `key: value` 行，即第一行的 `---` 与下一个 `---` 之间的内容。
/// 文件没有 front matter 时为空。
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

/// 把围栏代码块清空、行内代码换成占位符后的文字，这样 `[title](URL)` 这样的示例不算链接。行号保持不变。
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

/// GitHub 给标题生成的 slug：转成小写，去掉 `-` 和 `_` 以外的标点，空格变成 `-`。
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

/// 汉字旁的 ASCII 逗号，中间可以隔着右括号或左括号、强调标记或空格（`(可选绑定), 也`）。链接的目标不算，
/// 所以指向中文 Wiki 锚点的英文链接（`[contacts](…#联系人与群), …`）不算匹配，逗号前是中文的链接文字则仍然算。
#[test]
fn commas_next_to_chinese_are_full_width() {
    let han = r"[\u{4e00}-\u{9fff}]";
    let after_han = Regex::new(&format!(r"{han}[)\]」”’*_ ]*,")).unwrap();
    let before_han = Regex::new(&format!(r",[(\[「“‘*_ ]*{han}")).unwrap();
    let link_target = Regex::new(r"\]\([^)\s]*\)").unwrap();
    let mut problems = Vec::new();
    for file in doc_files() {
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

/// `.agents/skills` 中的每个目录都是 Agent Skills 格式的 skill：它的 `SKILL.md` 以 front matter 开头，
/// 其中 `name` 是目录名（由单个连字符连接的小写字母和数字，最多 64 个字符），`description`
/// 是一行不超过 1024 个字符的文字，智能体凭它决定什么时候用这个 skill。
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

/// `AGENTS.md` 和 `.claude/skills` 已检出为指向 `CLAUDE.md` 和 `.agents/skills` 的符号链接。
/// Windows 上的 Git 在没有开启开发者模式和 `core.symlinks` 时，把符号链接检出为内容是目标路径的普通文件；
/// 读取这个文件的智能体会读不到说明或 skill，又没有任何报错，所以用这项检查提醒。
#[test]
fn symlinks_are_checked_out_as_links() {
    let mut problems = Vec::new();
    for (link, target) in [
        ("AGENTS.md", "CLAUDE.md"),
        (".claude/skills", ".agents/skills"),
    ] {
        let resolved = std::fs::canonicalize(root().join(link)).ok();
        let expected = std::fs::canonicalize(root().join(target)).ok();
        if resolved.is_none() || resolved != expected {
            problems.push(format!("{link} is not a link to {target}"));
        }
    }
    assert!(
        problems.is_empty(),
        "{}\ngit checked them out as plain files: on Windows turn on Developer Mode, run \
         `git config --global core.symlinks true` and check them out again, see \
         https://github.com/334456777/WeFlow/wiki/开发指南#windows-上的符号链接",
        problems.join("\n")
    );
}
