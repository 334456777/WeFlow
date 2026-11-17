//! Files and folders only their owner may read (#812): the config file holds the database key, the image keys and
//! the token of the command API, and the cache holds decrypted images and voices. On Unix they are made with mode
//! 0600 (files) and 0700 (folders). On Windows they take the access of the folder they are made in, which for the
//! default places is the user's own profile.

use std::fs;
use std::io;
use std::path::Path;

/// Creates `dir` and its missing parents; each folder made here is its owner's only. Folders that are there already
/// keep their mode.
///
/// # Errors
/// As [`fs::create_dir_all`].
pub fn create_dir_all(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

/// [`fs::OpenOptions`] that make a new file its owner's only; a file that is there already keeps its mode.
#[must_use]
pub fn open_options() -> fs::OpenOptions {
    #[cfg_attr(not(unix), expect(unused_mut, reason = "the mode is set on Unix only"))]
    let mut options = fs::OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// Whether `path` lets others than its owner in: its group or anyone else may read, write or (a folder) enter it.
/// False when it cannot be read, and always on Windows, whose access lists this does not judge.
#[must_use]
pub fn open_to_others(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o077 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// The permission bits of `path`; 0 when it is not there.
    fn mode(path: &Path) -> u32 {
        fs::metadata(path).map_or(0, |meta| meta.permissions().mode() & 0o777)
    }

    #[test]
    fn what_is_made_here_is_the_owners_only() {
        let root = weflow_native::fixture::TestDir::new("private-files");
        let dir = root.join("a/b");
        create_dir_all(&dir).unwrap();
        assert_eq!(mode(&root.join("a")), 0o700);
        assert_eq!(mode(&dir), 0o700);
        assert!(!open_to_others(&dir));

        let file = dir.join("config.json");
        open_options()
            .write(true)
            .create_new(true)
            .open(&file)
            .unwrap()
            .write_all(b"{}")
            .unwrap();
        assert_eq!(mode(&file), 0o600);
        assert!(!open_to_others(&file));
    }

    #[test]
    fn a_mode_given_already_is_kept_and_seen() {
        let root = weflow_native::fixture::TestDir::new("private-files-kept");
        let dir = root.join("shared");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        create_dir_all(&dir).unwrap();
        assert_eq!(mode(&dir), 0o755);
        assert!(open_to_others(&dir));

        let file = dir.join("config.json");
        fs::write(&file, b"{}").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(open_to_others(&file));
        assert!(!open_to_others(&dir.join("missing")));
    }
}
