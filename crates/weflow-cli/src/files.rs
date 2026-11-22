//! Writing the files of a command: JSON, text and CSV, and the folders they go into.

use std::path::{Path, PathBuf};

use serde_json::Value;
use weflow_core::error::{AppError, AppResult};
use weflow_core::export_msg::csv_cell;

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
    if out
        .extension()
        .is_some_and(|ext| !ext.eq_ignore_ascii_case("zip"))
    {
        return Err(AppError::usage(
            "--zip writes a zip: give --out a folder or a .zip file",
        ));
    }
    Ok(export_path(out, default_name))
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
    write_text_file(path, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
