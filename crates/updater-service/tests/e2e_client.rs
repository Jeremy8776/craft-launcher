//! Client half of the service end-to-end test: talks to the real running service over the real
//! pipe, as whatever user runs it, and prints one `REPLY {json}` line per request.
//! Ignored by default; `e2e_client.py` runs it while the service is up.

#![cfg(windows)]

use std::time::Duration;

use artcraft_launcher_updater_service::pipe::{PIPE_PATH, request};

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
