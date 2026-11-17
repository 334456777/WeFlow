//! A command that cannot write where it was told says where and why (#816): the path, and what the system answered.

mod common;

use std::path::{Path, PathBuf};

use weflow_core::error::AppError;
use weflow_core::export_msg::names::DisplayPref;
use weflow_core::services::MessageExportRequest;

/// A folder that cannot be made, since a file is where its parent would be, and what the system says to making it.
fn blocked(root: &Path) -> (PathBuf, String) {
    let file = root.join("a-file");
    std::fs::write(&file, b"in the way").unwrap();
    let folder = file.join("sub");
    let why = std::fs::create_dir_all(&folder).unwrap_err().to_string();
    (folder, why)
}

fn assert_says(err: &AppError, path: &Path, why: &str, what: &str) {
    let path = path.display().to_string();
    assert!(
        err.message.contains(&path) && err.message.contains(why),
        "{what}: `{}` should name {path} and say `{why}`",
        err.message
    );
}

#[test]
fn a_message_export_that_cannot_write_says_where_and_why() {
    let (root, hub) = common::mock_hub("errors-export");
    let (folder, why) = blocked(&root);
    for format in [
        "txt",
        "json",
        "arkme-json",
        "chatlab",
        "chatlab-jsonl",
        "excel",
        "weclone",
        "html",
        "sql",
    ] {
        let request = MessageExportRequest {
            session_id: "wxid_bob".into(),
            format: format.into(),
            start: None,
            end: None,
            sender: None,
            display_pref: DisplayPref::Remark,
            excel_compact: false,
        };
        let err = hub
            .export_messages(&request, &folder.join(format!("out.{format}")))
            .unwrap_err();
        assert_says(&err, &folder, &why, format);
    }
}

#[test]
fn a_group_export_that_cannot_write_says_where_and_why() {
    let (root, hub) = common::mock_hub("errors-group");
    let (folder, why) = blocked(&root);
    for name in ["members.csv", "members.xlsx"] {
        let err = hub
            .group_export_members("room1@chatroom", &folder.join(name))
            .unwrap_err();
        assert_says(&err, &folder, &why, name);
    }
}
