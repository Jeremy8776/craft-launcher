//! A real per-machine update, with the engine's real command line, real `SystemRunner` and the
//! real Windows Installer. Ignored by default: it installs and removes a renamed test product
//! (`IptzUpdaterE2E`, never a real Craft App) under Program Files and so needs an elevated prompt.
//!
//! Run by `e2e/e2e.ps1` with `E2E_MSI_DIR` pointing at the MSIs built by `build_msis.py`.
//! The fakes are the things this test deliberately does not exercise: which app a product belongs
//! to (a real Craft App would have to be installed per-machine), the download, and the publisher's
//! certificate (the test MSIs are unsigned). Signature checking has its own real tests in
//! `wintrust.rs`. What IS real here: Windows Installer, the command line, and the service's reading
//! of Windows Installer's machine records for a genuine per-machine install.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;

use artcraft_launcher_core::CraftApp;
use artcraft_launcher_updater_service::engine::{Engine, Fetched, Fetcher};
use artcraft_launcher_updater_service::error::{Outcome, Refusal};
use artcraft_launcher_updater_service::install::SystemRunner;
use artcraft_launcher_updater_service::machine::{Installed, Machine, Registry};
use artcraft_launcher_updater_service::publisher::{SignatureCheck, Signatures};

/// The product codes `build_msis.py` gives the test MSIs.
const CODE_122: &str = "{5E4A7C10-1B22-4D33-8E44-0A5B6C7D8E22}";
const CODE_123: &str = "{5E4A7C10-1B22-4D33-8E44-0A5B6C7D8E23}";

/// Says the test product is installed at `version` in `folder`.
struct Says {
    version: &'static str,
    folder: PathBuf,
}
impl Machine for Says {
    fn installed(&self, _: &CraftApp) -> Option<Installed> {
        Some(Installed { version: semver::Version::parse(self.version).ok()?, folder: Some(self.folder.clone()) })
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

fn program_files() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramW6432").unwrap_or_else(|| r"C:\Program Files".into()))
}

fn install_folder() -> PathBuf {
    program_files().join("IptzUpdaterE2E")
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
        for (folder, version) in [("v123", "1.2.3"), ("v122", "1.2.2")] {
            let msi = self.0.join(folder).join(format!("cadcraft-{version}-windows-x64.msi"));
            let _ = Command::new(msiexec()).arg("/x").arg(msi).args(["/qn", "/norestart"]).status();
        }
    }
}

fn is_elevated() -> bool {
    Command::new("net").arg("session").output().is_ok_and(|o| o.status.success())
}

fn version(text: &str) -> Option<semver::Version> {
    semver::Version::parse(text).ok()
}

#[test]
#[ignore = "installs a renamed test product under Program Files: needs an elevated prompt"]
fn updates_a_per_machine_install_with_the_real_command_line() {
    assert!(is_elevated(), "run this from an elevated prompt (e2e/e2e.ps1 does)");
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

    // The service's own reading of Windows Installer's machine records, on a genuine per-machine
    // install. This is the detection that decides whether an app is the service's to update.
    assert_eq!(Registry::machine_version(CODE_122), version("1.2.2"), "a real per-machine install must be seen as per-machine");
    let paths = Registry::machine_component_paths(CODE_122);
    assert!(paths.iter().any(|p| p.to_ascii_lowercase().ends_with(r"iptzupdatere2e\version.txt")), "component paths: {paths:?}");
    assert_eq!(Registry::machine_version(CODE_123), None, "a product that is not installed must not be seen");

    let staging = tempfile::tempdir().unwrap();
    let (fetch, signer) = (CopyFrom(msis.clone()), SignedByUs);
    let roots = [program_files()];
    let run = |installed: &'static str, folder: PathBuf, body: &[u8]| {
        let machine = Says { version: installed, folder };
        let engine = Engine {
            staging_root: staging.path(),
            program_files: &roots,
            msiexec: &msiexec(),
            machine: &machine,
            fetcher: &fetch,
            signatures: &signer,
            runner: &SystemRunner,
        };
        engine.handle(body)
    };

    // The update: the engine's own command line (with INSTALLFOLDER) run by the real Windows Installer.
    assert_eq!(run("1.2.2", install_folder(), br#"{"app":"cadcraft","version":"1.2.3"}"#), Ok(Outcome::Installed));
    assert_eq!(installed_version().as_deref(), Some("IptzUpdaterE2E 1.2.3"), "the files were not updated in place");
    assert_eq!(Registry::machine_version(CODE_123), version("1.2.3"), "the machine record did not move to 1.2.3");
    assert_eq!(Registry::machine_version(CODE_122), None, "the old product is still registered");

    // Windows Installer's own downgrade refusal, as a second line behind the engine's check. The
    // engine is told (wrongly) that 1.2.1 is installed so that it gets as far as running msiexec.
    assert_eq!(run("1.2.1", install_folder(), br#"{"app":"cadcraft","version":"1.2.2"}"#), Ok(Outcome::Failed(1603)));
    assert_eq!(installed_version().as_deref(), Some("IptzUpdaterE2E 1.2.3"), "a downgrade went through");

    // The engine's own refusal of the same, with the true installed version.
    assert_eq!(run("1.2.3", install_folder(), br#"{"app":"cadcraft","version":"1.2.2"}"#), Err(Refusal::NotNewer));

    // An app recorded outside Program Files is declined, not installed and not failed: nothing runs.
    let elsewhere = tempfile::tempdir().unwrap();
    let declined = run("1.2.3", elsewhere.path().to_path_buf(), br#"{"app":"cadcraft","version":"1.2.4"}"#);
    assert!(matches!(declined, Err(ref r) if r.is_decline()), "{declined:?}");
    assert_eq!(installed_version().as_deref(), Some("IptzUpdaterE2E 1.2.3"));
}
