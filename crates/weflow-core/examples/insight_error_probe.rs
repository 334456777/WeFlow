//! Local synthetic HTTP matrix. No provider or WeChat account is contacted.
use std::io::{Read, Write};
use std::net::TcpListener;
use weflow_core::insight::call_api;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    for (status, response) in [
        (
            200,
            r#"{"choices":[{"message":{"content":"synthetic answer"}}]}"#,
        ),
        (
            401,
            r#"{"error":{"message":"synthetic invalid key","code":"invalid_api_key"}}"#,
        ),
        (
            429,
            r#"{"error":{"message":"synthetic rate limit","code":"rate_limit"}}"#,
        ),
        (
            500,
            r#"{"choices":[{"message":{"content":"synthetic error-status answer"}}]}"#,
        ),
        (200, r#"{"choices":[{"message":{"content":" "}}]}"#),
        (502, "synthetic non-JSON upstream error"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let base = format!("http://{}/v1", listener.local_addr()?);
        let server = std::thread::spawn(move || -> std::io::Result<String> {
            let (mut stream, _) = listener.accept()?;
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            let mut wanted = None;
            loop {
                let n = stream.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some(pos) = text.find("\r\n\r\n") {
                    let len = text[..pos]
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    wanted = Some(pos + 4 + len);
                }
                if wanted.is_some_and(|len| request.len() >= len) {
                    break;
                }
            }
            write!(stream, "HTTP/1.1 {status} Synthetic\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len())?;
            Ok(String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap_or("")
                .to_string())
        });
        let result = call_api(
            &base,
            "synthetic-key",
            "synthetic-model",
            &[("user", "synthetic prompt")],
            2_000,
            100,
        )
        .await;
        let path = server.join().unwrap()?;
        println!(
            "{}",
            serde_json::json!({"status":status,"request_line":path,"result":result})
        );
    }
    Ok(())
}
