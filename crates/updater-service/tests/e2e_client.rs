//! Client half of the service end-to-end test: talks to the real running service over the real
//! pipe, as whatever user runs it, and prints one `REPLY {json}` line per request.
//! Ignored by default; `e2e_client.py` runs it while the service is up.

#![cfg(windows)]

use std::time::Duration;

use artcraft_launcher_updater_client::pipe::{PIPE_PATH, request};

#[test]
#[ignore = "talks to a running updater service"]
fn asks_the_running_service() {
    let big = format!(r#"{{"app":"cadcraft","version":"9.9.9","pad":"{}"}}"#, "x".repeat(2000));
    let asks: [(&str, Vec<u8>); 7] = [
        ("not-installed", br#"{"app":"cadcraft","version":"9.9.9"}"#.to_vec()),
        ("unknown-app", br#"{"app":"notepad","version":"1.0.0"}"#.to_vec()),
        ("launcher", br#"{"app":"artcraft-launcher","version":"9.9.9"}"#.to_vec()),
        ("garbage", b"not json".to_vec()),
        ("path-field", br#"{"app":"cadcraft","version":"9.9.9","path":"C:\\evil.msi"}"#.to_vec()),
        ("oversize", big.into_bytes()),
        ("after", br#"{"app":"cadcraft","version":"9.9.9"}"#.to_vec()),
    ];
    for (label, body) in asks {
        let reply = request(PIPE_PATH, &body, Duration::from_secs(30)).unwrap_or_else(|e| panic!("{label}: {e}"));
        println!("REPLY {}", serde_json::json!({ "label": label, "reply": reply }));
    }
}

/// Asks the running service for one update, named by `E2E_APP` and `E2E_VERSION`, and prints the
/// reply. Used for the real-app run (`e2e/real_update.ps1`).
#[test]
#[ignore = "talks to a running updater service"]
fn asks_for_one_update() {
    let app = std::env::var("E2E_APP").unwrap_or_default();
    let version = std::env::var("E2E_VERSION").unwrap_or_default();
    let body = serde_json::json!({ "app": app, "version": version }).to_string();
    let reply = request(PIPE_PATH, body.as_bytes(), Duration::from_secs(900)).unwrap_or_else(|e| panic!("update: {e}"));
    println!("REPLY {}", serde_json::json!({ "label": "update", "reply": reply }));
}
