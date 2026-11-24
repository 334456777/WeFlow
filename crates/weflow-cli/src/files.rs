//! Writing the files of a command: JSON, text and CSV, and the folders they go into.

use std::path::{Path, PathBuf};

use serde_json::Value;
use weflow_core::error::{AppError, AppResult};
use weflow_core::export_msg::{csv_cell, write_csv};

pub(crate) fn write_json_file(path: &Path, value: &Value) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            AppError::runtime(format!("failed to create {}: {err}", parent.display()))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| AppError::runtime(format!("failed to serialize JSON: {err}")))?;
    std::fs::write(path, bytes)
        .map_err(|err| AppError::runtime(format!("failed to write {}: {err}", path.display())))
}

pub(crate) fn export_path(out: &Path, default_name: &str) -> PathBuf {
    if out.extension().is_some() {
        out.to_path_buf()
    } else {
        out.join(default_name)
    }
}

/// The zip `--zip` writes (#1040): `out` when it names a `.zip` file, `default_name` in the folder `out` otherwise.
///
/// # Errors
///
/// A usage error when `out` has another extension: a zip is not written under the name of a text, a page or a table.
pub(crate) fn zip_target(out: &Path, default_name: &str) -> AppResult<PathBuf> {
    zip_in(out, default_name)
        .ok_or_else(|| AppError::usage("--zip writes a zip: give --out a folder or a .zip file"))
}

/// The file `backup create` writes (#1134): `default_name` in `out` when `out` is a folder, one that is there already
/// or a name without an extension; `out` itself when it names a `.zip` file.
///
/// # Errors
///
/// A usage error when `out` names a file with another extension: a backup is a zip.
pub(crate) fn backup_target(out: &Path, default_name: &str) -> AppResult<PathBuf> {
    if out.is_dir() {
        return Ok(out.join(default_name));
    }
    zip_in(out, default_name)
        .ok_or_else(|| AppError::usage("a backup is a zip: give --out a folder or a .zip file"))
}

/// `out` when it names a `.zip` file, `default_name` in the folder `out` when it has no extension; `None` when it has
/// another one.
fn zip_in(out: &Path, default_name: &str) -> Option<PathBuf> {
    let other = out
        .extension()
        .is_some_and(|ext| !ext.eq_ignore_ascii_case("zip"));
    (!other).then(|| export_path(out, default_name))
}

/// The folder `path` goes into, made when it is not there: the writers of xlsx, HTML and SQL do not make it.
pub(crate) fn make_parent(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            AppError::runtime(format!("failed to create {}: {err}", parent.display()))
        })?;
    }
    Ok(())
}

pub(crate) fn write_text_file(path: &Path, text: &str) -> AppResult<()> {
    make_parent(path)?;
    std::fs::write(path, text)
        .map_err(|err| AppError::runtime(format!("failed to write {}: {err}", path.display())))
}

/// Writes `text`, the lines of a CSV file, to `path` as every CSV is written: UTF-8 after a byte order mark, so that
/// Excel reads it as UTF-8 (see [`write_csv`]).
pub(crate) fn write_csv_text(path: &Path, text: &str) -> AppResult<()> {
    write_csv(path, text).map_err(|err| AppError::runtime(format!("{err:#}")))
}

pub(crate) fn write_csv_file(path: &Path, value: &Value) -> AppResult<()> {
    let rows = value.as_array().ok_or_else(|| {
        AppError::runtime("CSV export expects an array; use --format json for nested data")
    })?;
    let mut headers = Vec::<String>::new();
    for row in rows {
        if let Some(object) = row.as_object() {
            for key in object.keys() {
                if !headers.contains(key) {
                    headers.push(key.clone());
                }
            }
        }
    }

    let mut out = String::new();
    if headers.is_empty() {
        out.push_str("index,value\n");
        for (idx, row) in rows.iter().enumerate() {
            out.push_str(&format!("{idx},{}\n", csv_cell(&row.to_string())));
        }
    } else {
        out.push_str(
            &headers
                .iter()
                .map(|header| csv_cell(header))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
        for row in rows {
            let Some(object) = row.as_object() else {
                continue;
            };
            out.push_str(
                &headers
                    .iter()
                    .map(|header| {
                        object
                            .get(header)
                            .map(|value| {
                                value
                                    .as_str()
                                    .map(ToString::to_string)
                                    .unwrap_or_else(|| value.to_string())
                            })
                            .map(|value| csv_cell(&value))
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            );
            out.push('\n');
        }
    }
    write_csv_text(path, &out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A list's CSV is the byte order mark, then the header of the rows' keys and a line per row (#1075).
    #[test]
    fn a_list_csv_starts_with_the_byte_order_mark() {
        let dir = weflow_native::fixture::TestDir::new("weflow-list-csv");
        let out = dir.join("contacts.csv");
        let list = json!([
            { "username": "wxid_bob", "remark": "老鲍" },
            { "username": "room@chatroom", "remark": "a, b" },
        ]);
        write_csv_file(&out, &list).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(bytes[..3], [0xEF, 0xBB, 0xBF]);
        let text = std::str::from_utf8(&bytes[3..]).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        let header: Vec<&str> = lines[0].split(',').collect();
        assert!(
            header.contains(&"username") && header.contains(&"remark"),
            "{text}"
        );
        assert!(
            lines[1].contains("老鲍") && lines[2].contains("\"a, b\""),
            "{text}"
        );
    }

    #[test]
    fn a_zip_goes_to_a_zip_file_or_into_a_folder() {
        let zip = |out: &str| zip_target(Path::new(out), "chat.zip");
        assert_eq!(zip("out/a.zip").unwrap(), Path::new("out/a.zip"));
        assert_eq!(zip("out/A.ZIP").unwrap(), Path::new("out/A.ZIP"));
        assert_eq!(zip("out").unwrap(), Path::new("out/chat.zip"));
        assert_eq!(
            zip("out/folder/").unwrap(),
            Path::new("out/folder/chat.zip")
        );
    }

    #[test]
    fn a_zip_is_not_named_like_another_kind_of_file() {
        for out in [
            "chat.txt",
            "out/chat.html",
            "out/chat.json",
            "out/a.zip.txt",
        ] {
            let err = zip_target(Path::new(out), "chat.zip").unwrap_err();
            assert_eq!((err.code.as_str(), err.exit_code), ("invalid_arguments", 2));
        }
    }

    /// A backup goes into a folder that is there, whatever its name, as into a name without an extension (#1134).
    #[test]
    fn a_backup_goes_to_a_zip_file_or_into_a_folder() {
        let dir = weflow_native::fixture::TestDir::new("weflow-backup-target");
        let dotted = dir.join("backups.2025");
        std::fs::create_dir(&dotted).unwrap();
        let named = dir.join("old.zip");
        std::fs::create_dir(&named).unwrap();
        for (out, want) in [
            (dir.to_path_buf(), dir.join("b.zip")),
            (dotted.clone(), dotted.join("b.zip")),
            (named.clone(), named.join("b.zip")),
            (dir.join("new"), dir.join("new").join("b.zip")),
            (dir.join("a.zip"), dir.join("a.zip")),
            (dir.join("A.ZIP"), dir.join("A.ZIP")),
        ] {
            assert_eq!(backup_target(&out, "b.zip").unwrap(), want, "{out:?}");
        }
        for out in ["backup.txt", "out/backup.json", "out/b.zip.bak"] {
            let err = backup_target(&dir.join(out), "b.zip").unwrap_err();
            assert_eq!(
                (err.code.as_str(), err.exit_code),
                ("invalid_arguments", 2),
                "{out}"
            );
        }
    }
}
