//! Real Authenticode checks through WinTrust, on real files. No installer is run.
//!
//! - A Microsoft-signed system file is validly signed, so it must pass the signature check and
//!   then be refused as the wrong publisher.
//! - A copy of an installed, signed Craft exe with one byte changed must fail the signature.
//! - An unsigned file must fail.
//! - The installed Craft exe (when this machine has one) must be accepted.

#![cfg(windows)]

use std::path::PathBuf;

use artcraft_launcher_updater_service::error::Refusal;
use artcraft_launcher_updater_service::publisher::{SignatureCheck, Signatures, require_publisher};
use artcraft_launcher_updater_service::wintrust::WinTrust;

fn trust() -> WinTrust {
    WinTrust::new().unwrap()
}

fn system_file(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into())).join(name)
}

/// The signed CADCraft exe this machine installed from a release, if there is one.
fn installed_craft_exe() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let exe = PathBuf::from(local).join(r"Programs\ArtCraft\CADCraft\cadcraft.exe");
    exe.is_file().then_some(exe)
}

#[test]
fn a_microsoft_file_is_signed_but_not_by_us() {
    let check = trust().check(&system_file("explorer.exe"));
    match &check {
        SignatureCheck::Valid { organization } => assert_eq!(organization.as_deref(), Some("Microsoft Corporation")),
        SignatureCheck::Invalid(why) => panic!("explorer.exe should verify: {why}"),
    }
    assert!(matches!(require_publisher(check), Err(Refusal::WrongPublisher(_))));
}

#[test]
fn an_unsigned_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain.msi");
    std::fs::write(&file, b"not an installer").unwrap();
    let check = trust().check(&file);
    assert!(matches!(check, SignatureCheck::Invalid(_)), "{check:?}");
    assert!(matches!(require_publisher(check), Err(Refusal::Unsigned(_))));
}

#[test]
fn the_installed_craft_exe_is_accepted() {
    let Some(exe) = installed_craft_exe() else { return };
    let check = trust().check(&exe);
    assert_eq!(require_publisher(check.clone()), Ok(()), "{check:?}");
}

#[test]
fn a_craft_exe_changed_after_signing_is_refused() {
    let Some(exe) = installed_craft_exe() else { return };
    let mut bytes = std::fs::read(&exe).unwrap();
    // Change one byte well inside the file (the signature block is at the end).
    let at = bytes.len() / 2;
    bytes[at] ^= 0xFF;
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("cadcraft.exe");
    std::fs::write(&copy, &bytes).unwrap();
    let check = trust().check(&copy);
    assert!(matches!(check, SignatureCheck::Invalid(_)), "a tampered file verified: {check:?}");
    assert!(matches!(require_publisher(check), Err(Refusal::Unsigned(_))));
}
