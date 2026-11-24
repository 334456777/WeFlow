//! Every English message the code gives an error constructor (`AppError::runtime("…")`, `ImageResult::fail("…", …)`,
//! or the template of a `format!` given to them) or an error the command API answers itself (`error_response`) has a
//! Chinese entry in the catalog, so that a run in Chinese does not show it in English.

use std::path::{Path, PathBuf};

use super::translate;

/// The constructors whose message is shown to a person: the type, the function, and whether the message comes after
/// an error code.
const CONSTRUCTORS: &[(&str, &str, bool)] = &[
    ("AppError", "runtime", false),
    ("AppError", "not_found", false),
    ("AppError", "usage", false),
    ("AppError", "config", false),
    ("AppError", "native", false),
    ("AppError", "new", true),
    ("ImageResult", "fail", false),
    // `ImageResult::fail` as its own methods call it
    ("Self", "fail", false),
];

/// The functions of the command API that answer an error themselves (#1090), and how many arguments come before the
/// message: `error_response(status, code, message)`, and those that give it a message of their own. Every literal in
/// the message's argument counts, as `busy_with(if task { "…" } else { "…" }, …)` gives one of two.
const ANSWERS: &[(&str, usize)] = &[("error_response", 2), ("bad_request", 0), ("busy_with", 0)];

/// Templates whose placeholders hold a phrase the catalog matches on, with a value for each placeholder in order: a
/// plain sample there would make a message the code never gives.
const FILLS: &[(&str, &[&str])] = &[
    ("the WebUI package is refused: {what}", &["`a` is a link"]),
    (
        "unknown media kind: {other}; use {kinds}",
        &["x", "image, voice, video, emoji, file, avatar or all"],
    ),
    (
        "invalid value '{text}' for {key}; use {}",
        &["x", "ai_model_api_format", "openai or anthropic"],
    ),
];

/// Messages that are not shown to a person, with the reason.
const NOT_SHOWN: &[(&str, &str)] = &[];

/// A token of Rust source, as far as finding the messages needs.
#[derive(Debug, PartialEq)]
enum Token {
    /// A string literal, its escapes decoded.
    Str(String),
    /// An identifier, a keyword or a number.
    Word(String),
    /// Any other character but white space.
    Punct(char),
}

/// The tokens of `source`, each with the line it starts on; comments and char literals are left out.
fn lex(source: &str) -> Vec<(usize, Token)> {
    let chars: Vec<char> = source.chars().filter(|c| *c != '\r').collect();
    let mut tokens = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let start = i;
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while chars.get(i).is_some_and(|c| *c != '\n') {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if c == '"' {
            let (text, end) = quoted(&chars, i + 1);
            tokens.push((line, Token::Str(text)));
            i = end;
        } else if c == '\'' {
            // a char literal, or the quote of a lifetime
            if next == Some('\\') {
                i += 3;
                while chars.get(i).is_some_and(|c| *c != '\'') {
                    i += 1;
                }
                i += 1;
            } else if chars.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                tokens.push((line, Token::Punct(c)));
                i += 1;
            }
        } else if c.is_alphanumeric() || c == '_' {
            while chars
                .get(i)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let hashes = chars[i..].iter().take_while(|c| **c == '#').count();
            let quote_after_hashes = chars.get(i + hashes) == Some(&'"');
            match word.as_str() {
                "r" | "br" if quote_after_hashes => {
                    let (text, end) = raw(&chars, i + hashes + 1, hashes);
                    tokens.push((line, Token::Str(text)));
                    i = end;
                }
                "b" if hashes == 0 && quote_after_hashes => {
                    let (text, end) = quoted(&chars, i + 1);
                    tokens.push((line, Token::Str(text)));
                    i = end;
                }
                _ => tokens.push((line, Token::Word(word))),
            }
        } else {
            if !c.is_whitespace() {
                tokens.push((line, Token::Punct(c)));
            }
            i += 1;
        }
        let end = i.min(chars.len());
        line += chars[start..end].iter().filter(|c| **c == '\n').count();
    }
    tokens
}

/// The text of the string literal whose body starts at `i`, and the index after its closing quote.
fn quoted(chars: &[char], mut i: usize) -> (String, usize) {
    let mut text = String::new();
    while let Some(&c) = chars.get(i) {
        i += 1;
        match c {
            '"' => break,
            '\\' => {
                let Some(&escaped) = chars.get(i) else {
                    break;
                };
                i += 1;
                match escaped {
                    'n' => text.push('\n'),
                    't' => text.push('\t'),
                    'r' => text.push('\r'),
                    '0' => text.push('\0'),
                    'x' => {
                        let end = (i + 2).min(chars.len());
                        let hex: String = chars[i..end].iter().collect();
                        text.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                        i = end;
                    }
                    'u' => {
                        let close = chars[i..]
                            .iter()
                            .position(|c| *c == '}')
                            .map_or(chars.len(), |p| i + p);
                        let hex: String = chars[(i + 1).min(close)..close].iter().collect();
                        text.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                        i = close + 1;
                    }
                    // a line continued: the line break and the indent after it are not in the text
                    '\n' => {
                        while chars.get(i).is_some_and(|c| c.is_whitespace()) {
                            i += 1;
                        }
                    }
                    other => text.push(other),
                }
            }
            other => text.push(other),
        }
    }
    (text, i)
}

/// The text of the raw string literal with `hashes` `#`s whose body starts at `i`, and the index after it.
fn raw(chars: &[char], start: usize, hashes: usize) -> (String, usize) {
    let closes = |at: usize| {
        chars[at] == '"'
            && chars[at + 1..]
                .iter()
                .take(hashes)
                .filter(|c| **c == '#')
                .count()
                == hashes
    };
    match (start..chars.len()).find(|at| closes(*at)) {
        Some(end) => (chars[start..end].iter().collect(), end + 1 + hashes),
        None => (
            chars[start.min(chars.len())..].iter().collect(),
            chars.len(),
        ),
    }
}

fn is_punct(token: Option<&(usize, Token)>, c: char) -> bool {
    matches!(token, Some((_, Token::Punct(p))) if *p == c)
}

fn is_word(token: Option<&(usize, Token)>, word: &str) -> bool {
    matches!(token, Some((_, Token::Word(w))) if w == word)
}

/// The index after the item that starts at `i`: after a `;` outside brackets, or after the braces of its body.
fn item_end(tokens: &[(usize, Token)], mut i: usize) -> usize {
    let mut depth = 0usize;
    while let Some((_, token)) = tokens.get(i) {
        i += 1;
        match token {
            Token::Punct('(' | '[') => depth += 1,
            Token::Punct(')' | ']') => depth = depth.saturating_sub(1),
            Token::Punct(';') if depth == 0 => break,
            Token::Punct('{') => {
                let mut braces = 1usize;
                while let Some((_, token)) = tokens.get(i) {
                    i += 1;
                    match token {
                        Token::Punct('{') => braces += 1,
                        Token::Punct('}') => braces -= 1,
                        _ => {}
                    }
                    if braces == 0 {
                        break;
                    }
                }
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    i
}

/// `tokens` without the items marked `#[cfg(test)]`: test modules and test helpers.
fn without_tests(tokens: Vec<(usize, Token)>) -> Vec<(usize, Token)> {
    let mut test = vec![false; tokens.len()];
    let mut i = 0;
    while i < tokens.len() {
        let marked = is_punct(tokens.get(i), '#')
            && is_punct(tokens.get(i + 1), '[')
            && is_word(tokens.get(i + 2), "cfg")
            && is_punct(tokens.get(i + 3), '(')
            && is_word(tokens.get(i + 4), "test")
            && is_punct(tokens.get(i + 5), ')')
            && is_punct(tokens.get(i + 6), ']');
        if marked {
            let end = item_end(&tokens, i + 7);
            test[i..end].fill(true);
            i = end;
        } else {
            i += 1;
        }
    }
    tokens
        .into_iter()
        .zip(test)
        .filter_map(|(token, test)| (!test).then_some(token))
        .collect()
}

/// A message given to one of the [`CONSTRUCTORS`].
#[derive(Debug)]
struct Message {
    /// The literal as written: the template of a `format!`, or the message itself.
    literal: String,
    /// The message: the template with its placeholders filled.
    text: String,
    /// The file and line.
    place: String,
}

/// The message a `format!` template gives with `fills` in its placeholders in order and `X` in the rest; `None` when
/// the template has no words of its own, only placeholders.
fn render(template: &str, fills: &[&str]) -> Option<String> {
    let mut text = String::new();
    let mut words = false;
    let mut filled = 0;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' | '}' if chars.peek() == Some(&c) => {
                chars.next();
                text.push(c);
            }
            '{' => {
                chars.by_ref().find(|c| *c == '}');
                text.push_str(fills.get(filled).copied().unwrap_or("X"));
                filled += 1;
            }
            c => {
                words |= c.is_alphabetic();
                text.push(c);
            }
        }
    }
    words.then_some(text)
}

/// Whether the tokens at `at` begin a `format!(`.
fn is_format(tokens: &[(usize, Token)], at: usize) -> bool {
    is_word(tokens.get(at), "format")
        && is_punct(tokens.get(at + 1), '!')
        && is_punct(tokens.get(at + 2), '(')
}

/// The literal given to one of the [`CONSTRUCTORS`] whose type is the word `ty` at `i`, and whether it is the
/// template of a `format!`; `None` when there is no such call there, or it is given no literal.
fn constructed(tokens: &[(usize, Token)], i: usize, ty: &str) -> Option<(String, bool)> {
    let &(_, _, coded) = CONSTRUCTORS
        .iter()
        .find(|(t, f, _)| *t == ty && is_word(tokens.get(i + 3), f))?;
    if !(is_punct(tokens.get(i + 1), ':')
        && is_punct(tokens.get(i + 2), ':')
        && is_punct(tokens.get(i + 4), '('))
    {
        return None;
    }
    let mut at = i + 5;
    if coded {
        if !(matches!(tokens.get(at), Some((_, Token::Str(_))))
            && is_punct(tokens.get(at + 1), ','))
        {
            return None;
        }
        at += 2;
    }
    let formatted = is_format(tokens, at);
    if formatted {
        at += 3;
    }
    match tokens.get(at) {
        Some((_, Token::Str(literal))) => Some((literal.clone(), formatted)),
        _ => None,
    }
}

/// The index of the `,` or the closing bracket that ends the argument starting at `at`.
fn argument_end(tokens: &[(usize, Token)], mut at: usize) -> usize {
    let mut depth = 0usize;
    while let Some((_, token)) = tokens.get(at) {
        match token {
            Token::Punct(')' | ']' | '}' | ',') if depth == 0 => break,
            Token::Punct('(' | '[' | '{') => depth += 1,
            Token::Punct(')' | ']' | '}') => depth -= 1,
            _ => {}
        }
        at += 1;
    }
    at
}

/// The literals given to a call of one of the [`ANSWERS`] whose name is at `i`, as the message after `skip` other
/// arguments, each with whether it is the template of a `format!`; none for the function's definition.
fn answered(tokens: &[(usize, Token)], i: usize, skip: usize) -> Vec<(String, bool)> {
    let defined = i
        .checked_sub(1)
        .is_some_and(|before| is_word(tokens.get(before), "fn"));
    if defined || !is_punct(tokens.get(i + 1), '(') {
        return Vec::new();
    }
    let mut start = i + 2;
    for _ in 0..skip {
        start = argument_end(tokens, start);
        if !is_punct(tokens.get(start), ',') {
            return Vec::new();
        }
        start += 1;
    }
    let borrowed = usize::from(is_punct(tokens.get(start), '&'));
    if is_format(tokens, start + borrowed) {
        return match tokens.get(start + borrowed + 3) {
            Some((_, Token::Str(template))) => vec![(template.clone(), true)],
            _ => Vec::new(),
        };
    }
    let end = argument_end(tokens, start);
    tokens
        .get(start..end)
        .unwrap_or_default()
        .iter()
        .filter_map(|(_, token)| match token {
            Token::Str(literal) => Some((literal.clone(), false)),
            _ => None,
        })
        .collect()
}

/// The [`Message`] of `literal` at `place`: the literal itself, or, the template of a `format!`, the template with
/// its placeholders filled; `None` for a template of placeholders alone.
fn message(literal: String, formatted: bool, place: String) -> Option<Message> {
    let text = if formatted {
        let fills = FILLS
            .iter()
            .find(|(template, _)| *template == literal)
            .map_or(&[][..], |(_, fills)| fills);
        render(&literal, fills)?
    } else {
        literal.clone()
    };
    Some(Message {
        literal,
        text,
        place,
    })
}

/// The messages `source` (the file at `place`) gives the [`CONSTRUCTORS`] and the [`ANSWERS`] outside its tests.
fn messages_in(place: &str, source: &str) -> Vec<Message> {
    let tokens = without_tests(lex(source));
    let mut messages = Vec::new();
    for (i, (line, token)) in tokens.iter().enumerate() {
        let Token::Word(word) = token else {
            continue;
        };
        let given = match ANSWERS.iter().find(|(name, _)| name == word) {
            Some(&(_, skip)) => answered(&tokens, i, skip),
            None => constructed(&tokens, i, word).into_iter().collect(),
        };
        messages.extend(given.into_iter().filter_map(|(literal, formatted)| {
            message(literal, formatted, format!("{place}:{line}"))
        }));
    }
    messages
}

/// The `.rs` files under `dir`, in order, but test files.
fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if path.is_dir() {
            rust_files(&path, files);
        } else if name.ends_with(".rs") && !name.ends_with("tests.rs") {
            files.push(path);
        }
    }
}

/// The places in `source` (the file at `place`) outside its tests that name one of the [`CONSTRUCTORS`] without
/// calling it, as `.map_err(AppError::usage)` does (#1154): the message is made elsewhere, as a `String` the scan
/// does not see, so nothing checks that the catalog has its Chinese.
fn named_without_a_message(place: &str, source: &str) -> Vec<String> {
    let tokens = without_tests(lex(source));
    tokens
        .iter()
        .enumerate()
        .filter_map(|(i, (line, token))| {
            let Token::Word(ty) = token else {
                return None;
            };
            let named = CONSTRUCTORS
                .iter()
                .any(|(t, f, _)| t == ty && is_word(tokens.get(i + 3), f))
                && is_punct(tokens.get(i + 1), ':')
                && is_punct(tokens.get(i + 2), ':');
            (named && !is_punct(tokens.get(i + 4), '(')).then(|| format!("{place}:{line}"))
        })
        .collect()
}

/// Each `.rs` file of the crates of the workspace but test files, as its place (`weflow-core/src/x.rs`) and source.
fn workspace_sources() -> Vec<(String, String)> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for krate in ["weflow-cli", "weflow-core", "weflow-native"] {
        rust_files(&crates.join(krate).join("src"), &mut files);
    }
    files
        .iter()
        .map(|path| {
            let source = std::fs::read_to_string(path).unwrap();
            let place = path
                .strip_prefix(&crates)
                .unwrap_or(path)
                .display()
                .to_string();
            (place.replace('\\', "/"), source)
        })
        .collect()
}

/// The messages the crates of the workspace give the [`CONSTRUCTORS`].
fn workspace_messages() -> Vec<Message> {
    workspace_sources()
        .iter()
        .flat_map(|(place, source)| messages_in(place, source))
        .collect()
}

/// A constructor is given its message where the message is made, so that the scan sees it (#1154).
#[test]
fn no_constructor_is_handed_a_message_made_elsewhere() {
    let named: Vec<String> = workspace_sources()
        .iter()
        .flat_map(|(place, source)| named_without_a_message(place, source))
        .collect();
    assert!(
        named.is_empty(),
        "give the constructor the message's literal (`AppError::usage(format!(\"…\"))`) rather than its name, so \
         that its Chinese is checked: {named:#?}"
    );
}

#[test]
fn finds_a_constructor_named_without_a_message() {
    let source = r#"
        fn parse(s: &str) -> AppResult<i64> {
            s.parse().map_err(AppError::usage)?;
            let fail = ImageResult::fail;
            Err(AppError::usage("called"))?;
            s.parse().map_err(|_| AppError::runtime(format!("called {s}")))
        }
        #[cfg(test)]
        mod tests {
            fn t() { let _ = x.map_err(AppError::runtime); }
        }
    "#;
    assert_eq!(
        named_without_a_message("x.rs", source),
        ["x.rs:3", "x.rs:4"]
    );
}

#[test]
fn every_message_given_to_an_error_is_in_the_catalog() {
    let messages = workspace_messages();
    // a scan that stopped finding the constructors or the answers would pass with nothing to check
    for (literal, file) in [
        ("the video has no thumbnail or cover", "reads.rs"),
        ("no such job", "serve_api/job_routes.rs"),
    ] {
        assert!(
            messages.iter().any(|m| m.literal == literal),
            "the scan found {} messages, but not the one of {file}",
            messages.len()
        );
    }
    let untranslated: Vec<String> = messages
        .iter()
        .filter(|m| !NOT_SHOWN.iter().any(|(literal, _)| *literal == m.literal))
        .filter(|m| translate(&m.text).is_none())
        .map(|m| format!("{}: {:?}", m.place, m.literal))
        .collect();
    assert!(untranslated.is_empty(), "no Chinese for {untranslated:#?}");
}

/// An entry of [`FILLS`] or [`NOT_SHOWN`] whose message the code no longer gives would never be used again.
#[test]
fn every_fill_and_exception_names_a_message_in_the_source() {
    let messages = workspace_messages();
    let listed = FILLS
        .iter()
        .map(|(literal, _)| *literal)
        .chain(NOT_SHOWN.iter().map(|(literal, _)| *literal));
    let stale: Vec<&str> = listed
        .filter(|literal| !messages.iter().any(|m| m.literal == *literal))
        .collect();
    assert!(stale.is_empty(), "no message in the source for {stale:#?}");
}

#[test]
fn finds_the_messages_but_those_of_tests() {
    let source = r##"
        // AppError::runtime("in a comment")
        fn run() -> AppResult<()> {
            let brace = '{';
            let quote = '"';
            let _ = r#"AppError::runtime("in a raw string")"#;
            Err(AppError::usage("plain"))?;
            Err(AppError::not_found(format!("templated {name}: {}", "x")))?;
            Err(AppError::new("busy", "coded \"quoted\" \
                 and continued", 9))?;
            Err(AppError::runtime(format!("{err:#}")))?;
            ImageResult::fail("image result", "not_found");
            Err(crate::error::AppError::config(
                "on its own line",
            ))
        }
        #[cfg(test)]
        fn helper() { AppError::runtime("test helper"); }
        #[cfg(test)]
        #[expect(clippy::unwrap_used, reason = "tests")]
        mod tests {
            fn t() { AppError::runtime("in a test {"); }
        }
        fn after() -> AppError { AppError::native("after the tests") }
        fn own() -> Self { Self::fail("by its own method", "not_found") }
        fn error_response(status: StatusCode, code: &str, message: &str) -> Response { todo!() }
        fn answers(task: bool) {
            error_response(StatusCode::NOT_FOUND, "not_found", "answered");
            super::error_response(StatusCode::OK, "busy", &format!("answered {what}"));
            bad_request(&message);
            busy_with(if task { "one" } else { "other" }, if task { "5" } else { "1" });
        }
    "##;
    let found: Vec<(String, String)> = messages_in("x.rs", source)
        .into_iter()
        .map(|m| (m.text, m.place))
        .collect();
    let want = [
        ("plain", "x.rs:7"),
        ("templated X: X", "x.rs:8"),
        ("coded \"quoted\" and continued", "x.rs:9"),
        ("image result", "x.rs:12"),
        ("on its own line", "x.rs:13"),
        ("after the tests", "x.rs:24"),
        ("by its own method", "x.rs:25"),
        ("answered", "x.rs:28"),
        ("answered X", "x.rs:29"),
        ("one", "x.rs:31"),
        ("other", "x.rs:31"),
    ];
    assert_eq!(
        found,
        want.map(|(text, place)| (text.to_string(), place.to_string()))
    );
}

#[test]
fn templates_take_their_fills_in_order() {
    assert_eq!(
        render("a {} b {name:#} c {{literal}}", &["one"]).as_deref(),
        Some("a one b X c {literal}")
    );
    assert_eq!(render("{err:#}", &[]), None);
}
