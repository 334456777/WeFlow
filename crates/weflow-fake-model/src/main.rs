//! `weflow-fake-model [--host <address>] [--port <port>] [--key <key>]`: runs the fake model API until it is stopped,
//! after printing one JSON line with its addresses, so that a script can point weflow at it.
use std::process::ExitCode;

use weflow_fake_model::{FakeModel, KEY, MODELS};

const USAGE: &str = "usage: weflow-fake-model [--host <address>] [--port <port>] [--key <key>]";

fn main() -> ExitCode {
    let mut host = "127.0.0.1".to_string();
    let mut port = "0".to_string();
    let mut key = KEY.to_string();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "--host" => &mut host,
            "--port" => &mut port,
            "--key" => &mut key,
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        };
        let Some(value) = args.next() else {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        };
        *slot = value;
    }
    let fake = match FakeModel::start_on(&format!("{host}:{port}"), &key) {
        Ok(fake) => fake,
        Err(err) => {
            eprintln!("weflow-fake-model: cannot listen on {host}:{port}: {err}");
            return ExitCode::FAILURE;
        }
    };
    let models: Vec<&str> = MODELS.iter().map(|m| m.id).collect();
    println!(
        "{}",
        serde_json::json!({
            "type": "fake_model_started",
            "openaiBaseUrl": fake.openai_base_url(),
            "anthropicBaseUrl": fake.anthropic_base_url(),
            "key": key,
            "models": models,
        })
    );
    loop {
        std::thread::park();
    }
}
