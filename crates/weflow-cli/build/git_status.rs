//! The `git status` with which `build.rs` tells a dirty checkout from a clean one.
//!
//! The file is included by `build.rs` and by `tests/build_script.rs` (`#[path]`), as build scripts take no part in
//! `cargo test`.

/// The arguments of the git command that prints nothing for a clean checkout and the modified tracked files otherwise.
///
/// `--no-optional-locks` keeps git from writing the stat data it refreshed back to the index. A checkout whose files
/// were written in the same second as the index has stale stat data, and the index is a file `build.rs` has cargo
/// watch: a write by the script itself reads as a change after the build and runs the script again.
pub const DIRTY_CHECK: [&str; 4] = [
    "--no-optional-locks",
    "status",
    "--porcelain",
    "--untracked-files=no",
];
