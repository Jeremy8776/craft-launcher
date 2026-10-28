//! The fetcher against a scripted loopback server. Nothing here touches the internet.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use artcraft_launcher_core::SUITE;
use sha2::{Digest, Sha256};

use super::*;

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Serves `files` (path -> body) on a loopback port until the test ends. Returns the base URL.
fn serve(files: HashMap<String, Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let files = Arc::new(files);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let files = Arc::clone(&files);
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                let text = String::from_utf8_lossy(&head);
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                let reply = match files.get(&path) {
                    Some(body) => {
                        let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                        r.extend_from_slice(body);
                        r
                    }
                    None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                };
                let _ = stream.write_all(&reply);
            });
        }
    });
    format!("http://127.0.0.1:{port}/")
}

fn cad() -> &'static CraftApp {
    SUITE.iter().find(|a| a.id == "cadcraft").unwrap()
}

/// A release listing `body` under every Windows architecture's file name, with `listed` as its hash.
fn release(listed: &str, body: &[u8]) -> HashMap<String, Vec<u8>> {
    let names = ["x64", "x86", "arm64"].map(|a| format!("cadcraft-1.2.3-windows-{a}.msi"));
    let sums: String = names.iter().map(|n| format!("{listed}  {n}\n")).collect();
    let mut files = HashMap::new();
    files.insert("/v1.2.3/SHA256SUMS.txt".to_string(), sums.into_bytes());
    for n in names {
        files.insert(format!("/v1.2.3/{n}"), body.to_vec());
    }
    files
}

fn version(v: &str) -> semver::Version {
    semver::Version::parse(v).unwrap()
}

#[test]
fn downloads_and_reports_the_published_checksum() {
    let body = b"pretend installer".to_vec();
    let base = serve(release(&sha(&body), &body));
    let dir = tempfile::tempdir().unwrap();
    let fetched = GitHubFetcher::with_mirror(&base).unwrap().fetch(cad(), &version("1.2.3"), dir.path()).unwrap();
    assert_eq!(fetched.expected_sha256, sha(&body));
    assert_eq!(std::fs::read(&fetched.file).unwrap(), body);
    assert_eq!(fetched.file.parent().unwrap(), dir.path());
    assert!(fetched.file.file_name().unwrap().to_string_lossy().starts_with("cadcraft-1.2.3-windows-"));
}

#[test]
fn refuses_bytes_that_do_not_match_the_published_checksum() {
    let base = serve(release(&sha(b"what the release promised"), b"what the server actually sent"));
    let dir = tempfile::tempdir().unwrap();
    let result = GitHubFetcher::with_mirror(&base).unwrap().fetch(cad(), &version("1.2.3"), dir.path());
    assert!(result.is_err(), "{result:?}");
    // Nothing that failed its checksum is left under the installer's name.
    assert!(std::fs::read_dir(dir.path()).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".msi")));
}

#[test]
fn a_missing_release_is_an_error() {
    let base = serve(HashMap::new());
    let dir = tempfile::tempdir().unwrap();
    let error = GitHubFetcher::with_mirror(&base).unwrap().fetch(cad(), &version("9.9.9"), dir.path()).unwrap_err();
    assert!(error.contains("no release"), "{error}");
}

#[test]
fn a_release_without_a_windows_installer_is_an_error() {
    let mut files = HashMap::new();
    files.insert("/v1.2.3/SHA256SUMS.txt".to_string(), format!("{}  cadcraft-1.2.3-linux-x86_64.deb\n", sha(b"x")).into_bytes());
    let base = serve(files);
    let dir = tempfile::tempdir().unwrap();
    let error = GitHubFetcher::with_mirror(&base).unwrap().fetch(cad(), &version("1.2.3"), dir.path()).unwrap_err();
    assert!(error.contains("no Windows installer"), "{error}");
}

#[test]
fn the_real_origin_only_uses_github_release_downloads() {
    let fetcher = GitHubFetcher::new().unwrap();
    let prefix = fetcher.prefix("storytold/cadcraft");
    assert_eq!(prefix, "https://github.com/storytold/cadcraft/releases/download/");
    assert!(fetcher.allowed("https://github.com/storytold/cadcraft/releases/download/v1.2.3/cadcraft-1.2.3-windows-x64.msi", &prefix));
    for bad in [
        "https://github.com/storytold/cadcraft/releases/download/../../../evil/x/releases/download/v1/a.msi",
        "https://github.com/evil/cadcraft/releases/download/v1.2.3/a.msi",
        "https://evil.example/storytold/cadcraft/releases/download/v1.2.3/a.msi",
        "http://github.com/storytold/cadcraft/releases/download/v1.2.3/a.msi",
        "https://github.com/storytold/cadcraft/releases/download/v1.2.3/a.msi?x=1",
    ] {
        assert!(!fetcher.allowed(bad, &prefix), "{bad}");
    }
}
