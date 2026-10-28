//! A real per-machine update, with the engine's real command line, real `SystemRunner` and the
//! real Windows Installer. Ignored by default: it installs and removes a renamed test product
//! (`IptzUpdaterE2E`, never a real Craft App) under Program Files and so needs an elevated prompt.
//!
//! Run by `updater-e2e/e2e.ps1` with `E2E_MSI_DIR` pointing at the MSIs built by `build_msis.py`.
//! The fakes are the three things this test deliberately does not exercise: what is installed
//! (a real Craft App would have to be installed), the download, and the publisher's certificate
//! (the test MSIs are unsigned). Signature checking has its own real tests in `wintrust.rs`.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;

use artcraft_launcher_core::CraftApp;
use artcraft_launcher_updater_service::engine::{Engine, Fetched, Fetcher};
use artcraft_launcher_updater_service::error::Outcome;
use artcraft_launcher_updater_service::install::SystemRunner;
use artcraft_launcher_updater_service::machine::Machine;
use artcraft_launcher_updater_service::publisher::{SignatureCheck, Signatures};

struct Installed(&'static str);
impl Machine for Installed {
    fn installed(&self, _: &CraftApp) -> Option<semver::Version> {
        semver::Version::parse(self.0).ok()
    }
}

/// "Downloads" by copying the test MSI for the requested version into staging.
struct CopyFrom(PathBuf);
impl Fetcher for CopyFrom {
    fn fetch(&self, _: &CraftApp, version: &semver::Version, staging: &Path) -> Result<Fetched, String> {
        let name = format!("cadcraft-{version}-windows-x64.msi");
        let from = self.0.join(format!("v{}", version.to_string().replace('.', ""))).join(&name);
        let file = staging.join(&name);
        std::fs::copy(&from, &file).map_err(|e| format!("{} ({e})", from.display()))?;
        let expected_sha256 = artcraft_launcher_updater_service::stage::sha256_hex(&file).map_err(|e| e.to_string())?;
        Ok(Fetched { file, expected_sha256 })
    }
}

struct SignedByUs;
impl Signatures for SignedByUs {
    fn check(&self, _: &Path) -> SignatureCheck {
        SignatureCheck::Valid { organization: Some("Learning Machines Inc".into()) }
    }
}

fn install_folder() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramW6432").unwrap_or_else(|| r"C:\Program Files".into())).join("IptzUpdaterE2E")
}

fn installed_version() -> Option<String> {
    std::fs::read_to_string(install_folder().join("version.txt")).ok().map(|t| t.trim().to_string())
}

fn msiexec() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into())).join(r"System32\msiexec.exe")
}

/// Removes the test product however the test ends.
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        for version in ["v123", "v122"] {
            let name = format!("cadcraft-{}-windows-x64.msi", if version == "v123" { "1.2.3" } else { "1.2.2" });
            let _ = Command::new(msiexec()).arg("/x").arg(self.0.join(version).join(name)).args(["/qn", "/norestart"]).status();
        }
    }
}

fn is_elevated() -> bool {
    Command::new("net").arg("session").output().is_ok_and(|o| o.status.success())
}

#[test]
#[ignore = "installs a renamed test product under Program Files: needs an elevated prompt"]
fn updates_a_per_machine_install_with_the_real_command_line() {
    assert!(is_elevated(), "run this from an elevated prompt (updater-e2e/e2e.ps1 does)");
    let msis = PathBuf::from(std::env::var_os("E2E_MSI_DIR").expect("E2E_MSI_DIR"));
    let _cleanup = Cleanup(msis.clone());
    assert_eq!(installed_version(), None, "an earlier test product is still installed");

    // Setup: the "old" version, installed per-machine the way the launcher's MSI route does.
    let setup = Command::new(msiexec())
        .arg("/i")
        .arg(msis.join("v122").join("cadcraft-1.2.2-windows-x64.msi"))
        .args(["/qn", "/norestart", "ALLUSERS=1"])
        .status()
        .unwrap();
    assert_eq!(setup.code(), Some(0));
    assert_eq!(installed_version().as_deref(), Some("IptzUpdaterE2E 1.2.2"));

    let staging = tempfile::tempdir().unwrap();
    let (fetch, signer) = (CopyFrom(msis.clone()), SignedByUs);
    let run = |installed: &'static str, body: &[u8]| {
        let machine = Installed(installed);
        let engine = Engine {
            staging_root: staging.path(),
            msiexec: &msiexec(),
            machine: &machine,
            fetcher: &fetch,
            signatures: &signer,
            runner: &SystemRunner,
        };
        engine.handle(body)
    };

    // The update: the engine's own command line, run by the real Windows Installer.
    assert_eq!(run("1.2.2", br#"{"app":"cadcraft","version":"1.2.3"}"#), Ok(Outcome::Installed));
    assert_eq!(installed_version().as_deref(), Some("IptzUpdaterE2E 1.2.3"), "the files were not updated");

    // Windows Installer's own downgrade refusal, as a second line behind the engine's check. The
    // engine is told (wrongly) that 1.2.1 is installed so that it gets as far as running msiexec.
    assert_eq!(run("1.2.1", br#"{"app":"cadcraft","version":"1.2.2"}"#), Ok(Outcome::Failed(1603)));
    assert_eq!(installed_version().as_deref(), Some("IptzUpdaterE2E 1.2.3"), "a downgrade went through");

    // And the engine's own refusal of the same, with the true installed version.
    assert!(run("1.2.3", br#"{"app":"cadcraft","version":"1.2.2"}"#).is_err());
}
