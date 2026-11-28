//! 在子进程中运行的命令：命令行、通过 stdin 交给子进程的页密钥、自行结束的命令，以及结果的 HTTP 状态。

use axum::http::StatusCode;
use serde_json::Value;

use crate::served::{CANCEL_ON_STDIN, KEYS_ON_STDIN};

/// 服务端的语言，写成 `WEFLOW_LANG` 的取值，传给子进程。
pub(super) fn server_lang() -> &'static str {
    match weflow_core::locale::current() {
        weflow_core::locale::Lang::Zh => "zh",
        weflow_core::locale::Lang::En => "en",
    }
}

/// 运行 `weflow <args>` 的命令：没有 stdin，捕获 stdout 和 stderr。
fn command(exe: &std::path::Path, args: &[String], lang: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args)
        .env("WEFLOW_LANG", lang)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // 请求或作业在命令结束之前没有了，就终止命令
        .kill_on_drop(true);
    cmd
}

/// `args` 运行的命令是否必须让它自己结束、而不能终止：被终止的命令会把钩子留在微信里，
/// 这样的命令有 `key db`（#287）和 `image auto-download start`（#809）。命令运行期间 stdin 保持打开；
/// stdin 关闭时（作业被取消、请求断开或服务停止），命令先把钩子撤出微信再结束。
pub(super) fn ends_by_itself(args: &[String]) -> bool {
    let words: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(String::as_str)
        .collect();
    matches!(
        words.as_slice(),
        ["key", "db", ..] | ["image", "auto-download", "start", ..]
    )
}

/// 启动 [`command`]，把服务端已经派生的数据库页密钥写进它的 stdin（[`weflow_core::page_keys::export_keys`]）：
/// 打开数据库时要先派生密钥（约 0.1 秒），命令不必再做一次。密钥只经过管道，不写进文件或环境变量。
pub(super) async fn spawn(
    exe: &std::path::Path,
    args: &[String],
    lang: &str,
) -> std::io::Result<tokio::process::Child> {
    let mut cmd = command(exe, args, lang);
    if ends_by_itself(args) {
        // 丢弃时不终止：丢弃它的 stdin 后，它把钩子撤出微信，再自己结束
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
        // 写入出错时忽略：没读密钥就结束的命令本来就不需要它们。写完后关闭管道，命令读到结尾
        let _ = tokio::io::AsyncWriteExt::write_all(&mut stdin, &keys).await;
    }
    Ok(child)
}

/// 在子进程中运行 `weflow <args>`，等它结束，返回它的输出。
pub(super) async fn run(
    exe: &std::path::Path,
    args: &[String],
    lang: &str,
) -> std::io::Result<std::process::Output> {
    let mut child = spawn(exe, args, lang).await?;
    // `wait_with_output` 会先关闭 stdin，这会让自行结束的命令结束（见 [`spawn`]）；所以先取出 stdin，
    // 保留到命令结束，或者随请求一起丢弃
    let stdin = child.stdin.take();
    let output = child.wait_with_output().await;
    drop(stdin);
    output
}

/// `--json` 文档对应的 HTTP 状态：先看错误码，再看退出码。
pub(super) fn status_for(doc: &Value, exit: i32) -> StatusCode {
    if exit == 0 && doc["success"] == Value::Bool(true) {
        return StatusCode::OK;
    }
    let error = &doc["error"];
    match error["code"].as_str().unwrap_or("") {
        "invalid_arguments" => StatusCode::BAD_REQUEST,
        "config_error" => StatusCode::CONFLICT,
        "native_error" => StatusCode::INTERNAL_SERVER_ERROR,
        "runtime_error" if error["details"]["failureKind"] == "not_found" => StatusCode::NOT_FOUND,
        "runtime_error" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => status_for_exit(exit),
    }
}

/// 只按退出码确定的 HTTP 状态。
pub(super) fn status_for_exit(exit: i32) -> StatusCode {
    match exit {
        0 => StatusCode::OK,
        2 => StatusCode::BAD_REQUEST,
        3 => StatusCode::CONFLICT,
        // 客户端关闭了请求（nginx 的 499）
        #[expect(clippy::expect_used, reason = "499 在状态码的取值范围内")]
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
    fn only_the_commands_that_hook_wechat_are_let_to_end_by_themselves() {
        let line = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert!(ends_by_itself(&line("--json --progress key db --save")));
        assert!(ends_by_itself(&line("--json key db --timeout 60")));
        assert!(ends_by_itself(&line("--json image auto-download start")));
        assert!(ends_by_itself(&line(
            "--json image auto-download start --whitelist a@chatroom"
        )));
        assert!(!ends_by_itself(&line("--json key image")));
        assert!(!ends_by_itself(&line("--json chat messages db")));
        assert!(!ends_by_itself(&line("--json image auto-download status")));
        assert!(!ends_by_itself(&line("--json image decrypt x")));
    }
}
