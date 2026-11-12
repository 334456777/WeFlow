//! Writing the files of a command: JSON, text and CSV, and the folders they go into.

use std::path::{Path, PathBuf};

use serde_json::Value;
use weflow_core::error::{AppError, AppResult};

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
            out.push_str(&format!("{idx},{}\n", csv_escape(&row.to_string())));
        }
    } else {
        out.push_str(
            &headers
                .iter()
                .map(|header| csv_escape(header))
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
                            .map(|value| csv_escape(&value))
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

pub(crate) fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}
