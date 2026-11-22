//! The parts of `build.rs` that are more than cargo directives (issues #1054 and #1065): the files of git it has cargo
//! watch, in a plain checkout and in a linked worktree where `.git` is a file, the `git status` that tells a dirty
//! checkout, and the build time it works out itself. Build scripts take no part in `cargo test`, so the modules are
//! included here.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use weflow_native::fixture::TestDir;

#[path = "../build/git_status.rs"]
mod git_status;
#[path = "../build/git_watch.rs"]
mod git_watch;
#[path = "../build/utc.rs"]
mod utc;

/// Runs git in `dir` with a committer of its own and without signing, whatever the developer's configuration says.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `<root>/repo`: a repository on the branch `main` with one commit, and `crates/weflow-cli` below it for the build
/// script's working directory.
fn repo(root: &Path) -> PathBuf {
    let dir = root.join("repo");
    fs::create_dir_all(dir.join("crates/weflow-cli")).unwrap();
    git(&dir, &["init", "--quiet"]);
    git(&dir, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    fs::write(dir.join("README"), "a file to track").unwrap();
    git(&dir, &["add", "README"]);
    git(&dir, &["commit", "--quiet", "--message", "first"]);
    dir
}

/// The paths of `paths` as they are on disk, sorted: git prints them with `/` and, on Windows, may spell the folders
/// in another case. A path that does not exist fails the test, which is how the watched paths are checked to exist.
fn on_disk(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = paths
        .into_iter()
        .map(|path| {
            fs::canonicalize(&path).unwrap_or_else(|e| panic!("{} is missing: {e}", path.display()))
        })
        .collect();
    paths.sort();
    paths
}

/// The bytes of each watched file, for telling whether a git command changed any of them; a folder reads as `None`.
fn snapshot(paths: &[PathBuf]) -> Vec<Option<Vec<u8>>> {
    paths.iter().map(|path| fs::read(path).ok()).collect()
}

#[test]
fn a_plain_checkout_watches_the_files_of_its_git_folder() {
    let root = TestDir::new("build-git");
    let repo = repo(&root);
    let git_dir = repo.join(".git");

    assert_eq!(
        on_disk(git_watch::watched_paths(&repo.join("crates/weflow-cli"))),
        on_disk([
            git_dir.join("HEAD"),
            git_dir.join("index"),
            git_dir.join("refs/tags"),
            git_dir.join("refs/heads/main"),
        ])
    );
}

#[test]
fn a_linked_worktree_watches_its_own_head_and_index_and_the_shared_refs() {
    let root = TestDir::new("build-git");
    let repo = repo(&root);
    let worktree = root.join("linked");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "side",
            worktree.to_str().unwrap(),
        ],
    );
    assert!(
        worktree.join(".git").is_file(),
        "a linked worktree's .git is a file"
    );
    let crate_dir = worktree.join("crates/weflow-cli");
    fs::create_dir_all(&crate_dir).unwrap();

    let git_dir = repo.join(".git");
    let own = git_dir.join("worktrees/linked");
    assert_eq!(
        on_disk(git_watch::watched_paths(&crate_dir)),
        on_disk([
            own.join("HEAD"),
            own.join("index"),
            git_dir.join("refs/tags"),
            git_dir.join("refs/heads/side"),
        ])
    );
}

#[test]
fn a_commit_in_a_worktree_changes_a_watched_file() {
    let root = TestDir::new("build-git");
    let repo = repo(&root);
    let worktree = root.join("linked");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "side",
            worktree.to_str().unwrap(),
        ],
    );
    let crate_dir = worktree.join("crates/weflow-cli");
    fs::create_dir_all(&crate_dir).unwrap();

    // `HEAD` keeps naming the branch, so it is the branch's file that tells of the commit
    let watched = git_watch::watched_paths(&crate_dir);
    let before = snapshot(&watched);
    git(
        &worktree,
        &["commit", "--quiet", "--allow-empty", "--message", "second"],
    );
    assert_ne!(snapshot(&watched), before);
}

#[test]
fn a_detached_head_is_watched_by_the_file_that_holds_it() {
    let root = TestDir::new("build-git");
    let repo = repo(&root);
    git(&repo, &["checkout", "--quiet", "--detach"]);
    let crate_dir = repo.join("crates/weflow-cli");
    let git_dir = repo.join(".git");

    let watched = git_watch::watched_paths(&crate_dir);
    assert_eq!(
        on_disk(watched.clone()),
        on_disk([
            git_dir.join("HEAD"),
            git_dir.join("index"),
            git_dir.join("refs/tags"),
        ])
    );
    let before = snapshot(&watched);
    git(
        &repo,
        &["commit", "--quiet", "--allow-empty", "--message", "second"],
    );
    assert_ne!(snapshot(&watched), before);
}

#[test]
fn packed_refs_are_watched_once_they_exist_and_a_branch_that_is_only_packed_is_not() {
    let root = TestDir::new("build-git");
    let repo = repo(&root);
    let crate_dir = repo.join("crates/weflow-cli");
    let packed = repo.join(".git/packed-refs");
    assert!(!packed.exists(), "a new repository has no packed-refs");
    assert!(
        !git_watch::watched_paths(&crate_dir)
            .iter()
            .any(|path| path.ends_with("packed-refs")),
        "a path that does not exist is not watched"
    );

    git(&repo, &["pack-refs", "--all"]);
    let watched = on_disk(git_watch::watched_paths(&crate_dir));
    assert!(watched.contains(&on_disk([packed])[0]), "{watched:?}");
    assert!(
        !watched.iter().any(|path| path.ends_with("refs/heads/main")),
        "the branch lives in packed-refs now: {watched:?}"
    );
}

#[test]
fn nothing_is_watched_where_git_has_no_repository() {
    let root = TestDir::new("build-git");
    // a `.git` that is no repository, so that a repository above the temp folder does not answer
    fs::write(root.join(".git"), "not a gitfile").unwrap();
    assert!(git_watch::watched_paths(&root).is_empty());
}

#[test]
fn git_dirs_are_two_lines_and_a_relative_one_is_taken_from_the_working_directory() {
    let dir = Path::new("work/crates/weflow-cli");
    assert_eq!(
        git_watch::dirs_from(dir, "../../.git/worktrees/w\n../../.git"),
        Some((dir.join("../../.git/worktrees/w"), dir.join("../../.git")))
    );
    let absolute = std::env::temp_dir().join(".git");
    let line = absolute.to_str().unwrap();
    assert_eq!(
        git_watch::dirs_from(dir, &format!("{line}\n{line}")),
        Some((absolute.clone(), absolute))
    );
    // a git before 2.31 prints the flag it does not know back as a first line
    assert_eq!(
        git_watch::dirs_from(dir, "--path-format=absolute\n.git\n.git"),
        None
    );
    assert_eq!(git_watch::dirs_from(dir, ".git"), None);
    assert_eq!(git_watch::dirs_from(dir, ""), None);
}

/// What the build script's dirty check prints in `dir`, trimmed as the script does.
fn dirty_check(dir: &Path) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(git_status::DIRTY_CHECK)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        git_status::DIRTY_CHECK,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Sets the modification time of the file `path`, leaving its content as it is.
fn set_modified(path: &Path, time: SystemTime) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

#[test]
fn the_dirty_check_leaves_the_index_as_it_is_for_a_file_whose_time_changed() {
    let root = TestDir::new("build-git");
    let repo = repo(&root);
    let index = repo.join(".git/index");

    // the stat data in the index no longer fits the file, as in a checkout written a moment after the index; the
    // index gets a time of its own so that a write to it shows without waiting for the clock to move on
    let file_time = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    let index_time = UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    set_modified(&repo.join("README"), file_time);
    set_modified(&index, index_time);

    assert_eq!(dirty_check(&repo), "", "the content is the committed one");
    assert_eq!(
        fs::metadata(&index).unwrap().modified().unwrap(),
        index_time,
        "the dirty check wrote the index, which `build.rs` has cargo watch"
    );
}

#[test]
fn the_dirty_check_names_a_file_whose_content_changed() {
    let root = TestDir::new("build-git");
    let repo = repo(&root);
    assert_eq!(dirty_check(&repo), "", "a clean checkout prints nothing");

    fs::write(repo.join("README"), "another content").unwrap();
    let answer = dirty_check(&repo);
    assert!(answer.contains("README"), "{answer:?}");
}

/// The stamp of `secs` after the epoch.
fn stamp(secs: u64) -> String {
    utc::utc_stamp(UNIX_EPOCH + Duration::from_secs(secs)).unwrap()
}

#[test]
fn the_build_time_is_the_utc_minute_as_date_prints_it() {
    // each with what `date -u -d @<secs> '+%Y-%m-%d %H:%MZ'` prints
    for (secs, expected) in [
        (0, "1970-01-01 00:00Z"),
        (59, "1970-01-01 00:00Z"),
        (60, "1970-01-01 00:01Z"),
        (86_399, "1970-01-01 23:59Z"),
        (86_400, "1970-01-02 00:00Z"),
        (951_782_400, "2000-02-29 00:00Z"),
        (951_868_799, "2000-02-29 23:59Z"),
        (1_583_020_800, "2020-03-01 00:00Z"),
        (1_709_164_800, "2024-02-29 00:00Z"),
        (1_709_251_199, "2024-02-29 23:59Z"),
        (1_767_225_540, "2025-12-31 23:59Z"),
        (1_784_567_890, "2026-07-20 17:18Z"),
        (4_102_444_800, "2100-01-01 00:00Z"),
        (4_107_542_399, "2100-02-28 23:59Z"),
        (4_107_542_400, "2100-03-01 00:00Z"),
        (253_402_300_799, "9999-12-31 23:59Z"),
    ] {
        assert_eq!(stamp(secs), expected, "{secs} seconds after the epoch");
    }
}

/// Days since 1970-01-01 of a Gregorian date (Howard Hinnant's `days_from_civil`), the inverse of what the stamp does.
fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    let year = year - u64::from(month <= 2);
    let era = year / 400;
    let year_of_era = year % 400;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[test]
fn the_build_time_of_every_day_from_1970_to_2400_reads_back_as_that_day() {
    let mut previous = String::new();
    for days in 0..=(2400 - 1970) * 366 {
        let secs = days * 86_400 + 13 * 3_600 + 7 * 60 + 42;
        let text = stamp(secs);
        assert!(text > previous, "{text} follows {previous}");
        let (year, rest) = text.split_once('-').unwrap();
        let (month, rest) = rest.split_once('-').unwrap();
        let (day, time) = rest.split_once(' ').unwrap();
        assert_eq!(time, "13:07Z", "{text}");
        let (year, month, day): (u64, u64, u64) = (
            year.parse().unwrap(),
            month.parse().unwrap(),
            day.parse().unwrap(),
        );
        assert!(
            (1..=12).contains(&month) && (1..=31).contains(&day),
            "{text}"
        );
        assert_eq!(days_from_civil(year, month, day), days, "{text}");
        previous = text;
    }
}

#[test]
fn the_build_time_of_now_has_the_shape_of_the_stamp() {
    let text = utc::utc_stamp(SystemTime::now()).unwrap();
    let bytes = text.as_bytes();
    assert_eq!(bytes.len(), 17, "{text}");
    for (at, byte) in bytes.iter().enumerate() {
        match at {
            4 | 7 => assert_eq!(*byte, b'-', "{text}"),
            10 => assert_eq!(*byte, b' ', "{text}"),
            13 => assert_eq!(*byte, b':', "{text}"),
            16 => assert_eq!(*byte, b'Z', "{text}"),
            _ => assert!(byte.is_ascii_digit(), "{text}"),
        }
    }
    assert!(text.as_str() >= "2026-", "{text}");
}

#[test]
fn there_is_no_build_time_before_the_epoch() {
    assert_eq!(utc::utc_stamp(UNIX_EPOCH - Duration::from_secs(1)), None);
}
