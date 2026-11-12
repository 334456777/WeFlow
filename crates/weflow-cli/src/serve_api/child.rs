//! Commands run in a child process: the command line, the keys handed over on stdin, and the HTTP status of
//! their outcome.

use axum::http::StatusCode;
use serde_json::Value;

/// The server's language as `WEFLOW_LANG` names it, for its child processes.
pub(super) fn server_lang() -> &'static str {
    match weflow_core::locale::current() {
        weflow_core::locale::Lang::Zh => "zh",
        weflow_core::locale::Lang::En => "en",
    }
}

/// `weflow <args>` with no stdin and both outputs captured.
fn command(exe: &std::path::Path, args: &[String], lang: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args)
        .env("WEFLOW_LANG", lang)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // the command is killed if its request or job goes away before it ends
        .kill_on_drop(true);
    cmd
}

/// Set for a command whose stdin carries the database keys the server derived (see [`spawn`]).
pub const KEYS_ON_STDIN: &str = "WEFLOW_KEYS_ON_STDIN";

/// Set for a command that ends by itself once its stdin ends (see [`ends_by_itself`]).
pub const CANCEL_ON_STDIN: &str = "WEFLOW_CANCEL_ON_STDIN";

/// Whether the command `args` runs must be let to end by itself rather than killed: `key db`, whose hook would be
/// left in WeChat (#287). Its stdin stays open while it runs; once it ends (the job cancelled, the request or the
/// server gone), the command takes its hook out of WeChat and ends with `user_interrupt`.
pub(super) fn ends_by_itself(args: &[String]) -> bool {
    let mut words = args.iter().filter(|a| !a.starts_with('-'));
    words.next().is_some_and(|w| w == "key") && words.next().is_some_and(|w| w == "db")
}

/// [`command`], started, with the database keys this server derived written to its stdin
/// ([`weflow_core::page_keys::export_keys`]): opening a database derives its key first (about 0.1 s), and the
/// command does not have to do it again. The keys go through the pipe only, never to a file or the environment.
pub(super) async fn spawn(
    exe: &std::path::Path,
    args: &[String],
    lang: &str,
) -> std::io::Result<tokio::process::Child> {
    let mut cmd = command(exe, args, lang);
    if ends_by_itself(args) {
        // not killed when dropped: dropping its stdin ends it, once its hook is out of WeChat
        cmd.stdin(std::process::Stdio::piped())
            .env(CANCEL_ON_STDIN, "1")
            .kill_on_drop(false);
        return cmd.spawn();
    }
    let keys = weflow_core::page_keys::export_keys();
    if !keys.is_empty() {
        cmd.stdin(std::process::Stdio::piped())
            .env(KEYS_ON_STDIN, "1");
    }
    let mut child = cmd.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // a command that ends without reading them did not need them; closing the pipe ends what it reads
        let _ = tokio::io::AsyncWriteExt::write_all(&mut stdin, &keys).await;
    }
    Ok(child)
}

pub(super) async fn run(
    exe: &std::path::Path,
    args: &[String],
    lang: &str,
) -> std::io::Result<std::process::Output> {
    let mut child = spawn(exe, args, lang).await?;
    // `wait_with_output` closes stdin first, which would end a command that ends by itself (see [`spawn`]); it
    // is held until the command ends, or dropped with the request
    let stdin = child.stdin.take();
    let output = child.wait_with_output().await;
    drop(stdin);
    output
}

/// The HTTP status of a `--json` document: by its error code first, then by the exit code.
pub(super) fn status_for(doc: &Value, exit: i32) -> StatusCode {
    if exit == 0 && doc["success"] == Value::Bool(true) {
        return StatusCode::OK;
    }
    let error = &doc["error"];
    match error["code"].as_str().unwrap_or("") {
        "invalid_arguments" => StatusCode::BAD_REQUEST,
        "config_error" => StatusCode::CONFLICT,
        "unsupported" => StatusCode::NOT_IMPLEMENTED,
        "native_error" => StatusCode::INTERNAL_SERVER_ERROR,
        "runtime_error" if error["details"]["failureKind"] == "not_found" => StatusCode::NOT_FOUND,
        "runtime_error" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => status_for_exit(exit),
    }
}

pub(super) fn status_for_exit(exit: i32) -> StatusCode {
    match exit {
        0 => StatusCode::OK,
        2 => StatusCode::BAD_REQUEST,
        3 => StatusCode::CONFLICT,
        // the client closed the request (nginx's 499)
        #[expect(clippy::expect_used, reason = "499 lies in the range of status codes")]
        130 => StatusCode::from_u16(499).expect("499 is a status code"),
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_follows_the_error_code_then_the_exit_code() {
        let fail = |code: &str, details: Value| json!({ "success": false, "error": { "code": code, "message": "m", "details": details } });
        assert_eq!(
            status_for(&json!({ "success": true, "data": 1 }), 0),
            StatusCode::OK
        );
        assert_eq!(
            status_for(&fail("invalid_arguments", Value::Null), 2),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status_for(&fail("config_error", Value::Null), 3),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status_for(&fail("unsupported", Value::Null), 4),
            StatusCode::NOT_IMPLEMENTED
        );
        assert_eq!(
            status_for(&fail("native_error", Value::Null), 4),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status_for(
                &fail("runtime_error", json!({ "failureKind": "not_found" })),
                1
            ),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status_for(&fail("runtime_error", Value::Null), 1),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status_for(&fail("user_interrupt", Value::Null), 130).as_u16(),
            499
        );
        assert_eq!(
            status_for(&fail("something_new", Value::Null), 3),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn only_key_db_is_let_to_end_by_itself() {
        let line = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert!(ends_by_itself(&line("--json --progress key db --save")));
        assert!(ends_by_itself(&line("--json key db --timeout 60")));
        assert!(!ends_by_itself(&line("--json key image")));
        assert!(!ends_by_itself(&line("--json chat messages db")));
    }
}
