//! Synthetic cache-refresh experiment; requires --features test-fixtures.
//! It never opens a user's account. All mutations stay in a new temp directory.
use std::fs::{File, FileTimes};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use weflow_native::fixture::{Fixture, SessionSpec, T0};
use weflow_native::native_db::NativeAccount;

fn main() -> anyhow::Result<()> {
    let root = std::env::temp_dir().join(format!(
        "weflow-refresh-probe-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    ));
    let f = Fixture::new(&root, "wxid_synthetic_ab12");
    let write = |summary| {
        f.session_db(&[SessionSpec {
            username: "wxid_synthetic_peer",
            summary,
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 1,
        }]);
    };
    write("synthetic A");
    let db = f.db_storage().join("session/session.db");
    let account = NativeAccount::new(f.db_storage(), &f.key_hex())?;
    let first = account.sessions()?;
    let stamp = std::fs::metadata(&db)?;
    write("synthetic B");
    assert_eq!(std::fs::metadata(&db)?.len(), stamp.len());
    let file = File::options().write(true).open(&db)?;
    file.set_times(FileTimes::new().set_modified(stamp.modified()?))?;
    let preserved = account.sessions()?;
    file.set_times(FileTimes::new().set_modified(stamp.modified()? + Duration::from_secs(2)))?;
    let refreshed = account.sessions()?;
    println!(
        "{}",
        serde_json::json!({
            "same_size_and_mtime_keeps_old_snapshot": first == preserved,
            "mtime_change_refreshes_snapshot": refreshed != first,
            "refreshed_summary_matches_synthetic_b": refreshed[0]["summary"] == "synthetic B"
        })
    );
    Ok(())
}
