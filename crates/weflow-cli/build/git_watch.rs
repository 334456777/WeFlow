//! The files of git that `build.rs` has cargo watch, because a change in them means another commit, tag or set of
//! modified files: found by asking git, as in a linked worktree `.git` is a file and the folder it names lies elsewhere.
//!
//! The file is included by `build.rs` and by `tests/build_script.rs` (`#[path]`), as build scripts take no part in
//! `cargo test`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// What `git <args>` prints when run in `dir`, trimmed; `None` when git is missing or fails, as it does outside a
/// repository.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// The existing files and folders whose change means another version (a tag) or another commit or state of the
/// checkout that holds `dir`; none outside a repository, where the version comes from Cargo.toml and never changes.
///
/// A worktree has a git folder of its own (`HEAD`, `index`) under the repository's, which holds the rest (`refs`,
/// `packed-refs`) for all its worktrees. Only paths that exist are listed: cargo runs a build script again on every
/// build for a `rerun-if-changed` path that does not, and a shallow or a freshly fetched repository has no
/// `packed-refs`.
pub fn watched_paths(dir: &Path) -> Vec<PathBuf> {
    let Some((own, common)) = git_dirs(dir) else {
        return Vec::new();
    };
    let mut paths = vec![
        own.join("HEAD"),
        own.join("index"),
        common.join("refs/tags"),
        common.join("packed-refs"),
    ];
    // a commit moves the branch `HEAD` names and leaves `HEAD` itself as it was
    if let Some(branch) = git(dir, &["symbolic-ref", "--quiet", "HEAD"]) {
        paths.push(common.join(branch));
    }
    paths.retain(|path| path.exists());
    paths
}

/// The git folder of the checkout that holds `dir` and the one it shares with the other worktrees of its repository
/// (the same folder when it has no other).
fn git_dirs(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    // `--path-format` is from git 2.31: an older git prints it back as a line of its own, and the relative answer
    // of the second try is then made absolute here
    let attempts: [&[&str]; 2] = [
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ],
        &["rev-parse", "--git-dir", "--git-common-dir"],
    ];
    attempts
        .iter()
        .find_map(|args| dirs_from(dir, &git(dir, args)?))
}

/// The two folders of the answer to `git rev-parse --git-dir --git-common-dir` run in `dir`, a relative one taken from
/// `dir`; `None` for an answer that does not have exactly these two lines.
pub fn dirs_from(dir: &Path, answer: &str) -> Option<(PathBuf, PathBuf)> {
    let mut lines = answer.lines();
    let (own, common) = (lines.next()?, lines.next()?);
    if lines.next().is_some() {
        return None;
    }
    Some((dir.join(own), dir.join(common)))
}
