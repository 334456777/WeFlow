//! `weflow` run with `sudo` (#855): reading WeChat's memory needs root on Linux and macOS, but the config and the
//! cache belong to the user who ran `sudo`, not to root. With an effective user of root and a non-zero `SUDO_UID`,
//! the home folder and the default config folder are that user's, as the password database names them (`sudo`
//! points `HOME` at root's). What root makes or replaces in WeFlow's home folder is handed back to that user when the
//! command ends, so that the user can still read and write it without `sudo`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::sync::MutexExt;

/// The user who ran `sudo weflow …`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
}

/// The user who ran this process with `sudo`, or `None` when it does not run as root under `sudo` (and always on
/// Windows).
pub fn caller() -> Option<&'static Caller> {
    static CALLER: OnceLock<Option<Caller>> = OnceLock::new();
    CALLER.get_or_init(find_caller).as_ref()
}

/// The home folder of the user this runs for: the caller's under `sudo`, else the current user's.
pub fn home_dir() -> Option<PathBuf> {
    caller()
        .map(|caller| caller.home.clone())
        .or_else(dirs::home_dir)
}

/// The folder of per-user config files of the user this runs for, as [`dirs::config_dir`] gives it for that user:
/// `Library/Application Support` in the home folder on macOS, `.config` elsewhere (the caller's `XDG_CONFIG_HOME` is
/// not known under `sudo`, which clears it).
pub fn config_dir() -> Option<PathBuf> {
    match caller() {
        Some(caller) if cfg!(target_os = "macos") => {
            Some(caller.home.join("Library/Application Support"))
        }
        Some(caller) => Some(caller.home.join(".config")),
        None => dirs::config_dir(),
    }
}

/// The folders [`adopt`] was asked about, each with the folder whose root-owned entries [`hand_back`] gives to the
/// caller (`None` for one that is not the caller's).
static ADOPTED: Mutex<Vec<(PathBuf, Option<PathBuf>)>> = Mutex::new(Vec::new());

/// Called before WeFlow makes one of its folders `dir` (the home folder, the cache folder): under `sudo`, when the
/// nearest part of `dir` that exists belongs to the caller, what root makes in `dir` from now on (and the folders made
/// to reach it) goes back to the caller at [`hand_back`]. A folder of root's, or of anyone else, is left as it is.
pub fn adopt(dir: &Path) {
    let Some(caller) = caller() else {
        return;
    };
    if ADOPTED
        .lock_or_panic()
        .iter()
        .any(|(asked, _)| asked == dir)
    {
        return;
    }
    let (existing, top_missing) = nearest_existing(dir);
    let top = (existing.and_then(owner) == Some(caller.uid))
        .then(|| top_missing.unwrap_or(dir).to_path_buf());
    ADOPTED.lock_or_panic().push((dir.to_path_buf(), top));
}

/// Gives what root owns in the folders [`adopt`] took to the caller of `sudo`, user and group. Symbolic links are
/// neither followed nor changed, and entries of other owners keep theirs. Failures are logged and skipped: the command
/// has done its work by then.
pub fn hand_back() {
    let mut tops: Vec<PathBuf> = std::mem::take(&mut *ADOPTED.lock_or_panic())
        .into_iter()
        .filter_map(|(_, top)| top)
        .collect();
    let Some(caller) = caller() else {
        return;
    };
    // a folder inside another is walked with it
    tops.sort();
    tops.dedup_by(|inner, outer| inner.starts_with(outer));
    for path in tops.iter().flat_map(|top| entries_owned_by(top, 0)) {
        if let Err(err) = change_owner(&path, caller) {
            tracing::warn!(
                "cannot give {} back to uid {}: {err}",
                path.display(),
                caller.uid
            );
        }
    }
}

/// The nearest of `dir` and its parents that exists, and the outermost one before it that does not.
fn nearest_existing(dir: &Path) -> (Option<&Path>, Option<&Path>) {
    let mut missing = None;
    for path in dir.ancestors() {
        if path.symlink_metadata().is_ok() {
            return (Some(path), missing);
        }
        missing = Some(path);
    }
    (None, missing)
}

/// The paths in `top` (`top` included) owned by `uid`, symbolic links left out and not followed.
#[cfg(unix)]
fn entries_owned_by(top: &Path, uid: u32) -> Vec<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    walkdir::WalkDir::new(top)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_type().is_symlink())
        .filter(|entry| entry.metadata().is_ok_and(|meta| meta.uid() == uid))
        .map(walkdir::DirEntry::into_path)
        .collect()
}

#[cfg(not(unix))]
fn entries_owned_by(_top: &Path, _uid: u32) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(unix)]
fn owner(path: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|meta| meta.uid())
}

#[cfg(not(unix))]
fn owner(_path: &Path) -> Option<u32> {
    None
}

#[cfg(unix)]
fn change_owner(path: &Path, caller: &Caller) -> std::io::Result<()> {
    std::os::unix::fs::lchown(path, Some(caller.uid), Some(caller.gid))
}

#[cfg(not(unix))]
fn change_owner(_path: &Path, _caller: &Caller) -> std::io::Result<()> {
    Ok(())
}

/// A `SUDO_UID` value; `sudo` run by root itself sets 0, which needs nothing handed back.
#[cfg(any(test, unix))]
fn caller_id(value: &str) -> Option<u32> {
    value.trim().parse().ok().filter(|id| *id != 0)
}

#[cfg(unix)]
fn find_caller() -> Option<Caller> {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } != 0 {
        return None;
    }
    let uid = caller_id(&std::env::var("SUDO_UID").ok()?)?;
    let (home, passwd_gid) = passwd_entry(uid)?;
    let gid = std::env::var("SUDO_GID")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(passwd_gid);
    Some(Caller { uid, gid, home })
}

#[cfg(not(unix))]
fn find_caller() -> Option<Caller> {
    None
}

/// The home folder and primary group of `uid` in the password database.
#[cfg(unix)]
fn passwd_entry(uid: u32) -> Option<(PathBuf, u32)> {
    use std::os::unix::ffi::OsStrExt;
    let mut buffer = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: `passwd` is plain C data, for which all zero bytes are a valid value.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    // SAFETY: `entry`, `buffer` and `result` are live locals and `buffer.len()` is the buffer's size; on success the
    // strings of `entry` point into `buffer`.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() || entry.pw_dir.is_null() {
        return None;
    }
    // SAFETY: `pw_dir` is a NUL-terminated string in `buffer`, which outlives this borrow.
    let dir = unsafe { std::ffi::CStr::from_ptr(entry.pw_dir) };
    let home = PathBuf::from(std::ffi::OsStr::from_bytes(dir.to_bytes()));
    (!home.as_os_str().is_empty()).then_some((home, entry.pw_gid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_unset_ids_are_no_caller() {
        assert_eq!(caller_id("1000"), Some(1000));
        assert_eq!(caller_id(" 501\n"), Some(501));
        assert_eq!(caller_id("0"), None, "sudo run by root");
        assert_eq!(caller_id(""), None);
        assert_eq!(caller_id("me"), None);
    }

    #[test]
    fn the_outermost_missing_folder_is_where_handing_back_starts() {
        let root = weflow_native::fixture::TestDir::new("sudo-nearest");
        let home = root.join("a/b/weflow");
        assert_eq!(
            nearest_existing(&home),
            (Some(&*root), Some(&*root.join("a")))
        );
        std::fs::create_dir_all(&home).unwrap();
        assert_eq!(nearest_existing(&home), (Some(&*home), None));
    }

    /// What is handed back: the entries of an owner, never through or including a symbolic link.
    #[cfg(unix)]
    #[test]
    fn entries_of_an_owner_leave_out_symbolic_links() {
        use std::os::unix::fs::MetadataExt;
        let root = weflow_native::fixture::TestDir::new("sudo-entries");
        let outside = weflow_native::fixture::TestDir::new("sudo-entries-outside");
        std::fs::write(outside.join("secret"), b"x").unwrap();
        let home = root.join("weflow");
        std::fs::create_dir_all(home.join("cache/Images")).unwrap();
        std::fs::write(home.join("config.json"), b"{}").unwrap();
        std::os::unix::fs::symlink(&*outside, home.join("cache/link")).unwrap();
        let me = std::fs::metadata(&*root).unwrap().uid();

        let mut found = entries_owned_by(&home, me);
        found.sort();
        assert_eq!(
            found,
            [
                home.clone(),
                home.join("cache"),
                home.join("cache/Images"),
                home.join("config.json"),
            ]
        );
        assert!(entries_owned_by(&home, me + 1).is_empty());
    }
}
