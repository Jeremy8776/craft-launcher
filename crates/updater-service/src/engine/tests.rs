//! Engine tests with a scripted fetcher, signer and runner. The signature check against real
//! signed files is in `tests/wintrust.rs`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use artcraft_launcher_core::CraftApp;

use super::*;
use crate::install::Command;
use crate::machine::Installed;
use crate::publisher::SignatureCheck;

const PUBLISHER: &str = "Learning Machines Inc";
const REQUEST: &[u8] = br#"{"app":"cadcraft","version":"1.2.3"}"#;

/// Puts `bytes` in the staging folder under `name`, and reports `sum` as the published checksum.
struct Fetch {
    name: &'static str,
    bytes: &'static [u8],
    sum: Option<String>,
    outside: bool,
    fail: bool,
}

impl Fetcher for Fetch {
    fn fetch(&self, _: &CraftApp, _: &semver::Version, staging: &Path) -> Result<Fetched, String> {
        if self.fail {
            return Err("offline".into());
        }
        let dir = if self.outside { staging.parent().unwrap().to_path_buf() } else { staging.to_path_buf() };
        let file = dir.join(self.name);
        std::fs::write(&file, self.bytes).map_err(|e| e.to_string())?;
        let expected_sha256 = self.sum.clone().unwrap_or_else(|| sha256_hex(&file).unwrap());
        Ok(Fetched { file, expected_sha256 })
    }
}

struct Signer(SignatureCheck);
impl Signatures for Signer {
    fn check(&self, _: &Path) -> SignatureCheck {
        self.0.clone()
    }
}

/// A stand-in for Program Files that every test shares: created once, never removed.
fn pf_root() -> PathBuf {
    let root = std::env::temp_dir().join("ArtCraftUpdaterTestPF");
    std::fs::create_dir_all(root.join("ArtCraft").join("CADCraft")).unwrap();
    root
}

/// What is installed per-machine, as a version string (`None`: not installed), in the app's
/// folder inside the stand-in Program Files.
struct Mach(Option<&'static str>);
impl Machine for Mach {
    fn installed(&self, _: &CraftApp) -> Option<Installed> {
        let folder = Some(pf_root().join("ArtCraft").join("CADCraft"));
        self.0.map(|v| Installed { version: semver::Version::parse(v).unwrap(), folder })
    }
}

/// Installed at version 1.2.2 in exactly `folder` (which may be missing, or outside Program Files).
struct MachAt(Option<PathBuf>);
impl Machine for MachAt {
    fn installed(&self, _: &CraftApp) -> Option<Installed> {
        Some(Installed { version: semver::Version::parse("1.2.2").unwrap(), folder: self.0.clone() })
    }
}

#[derive(Default)]
struct Runs {
    seen: RefCell<Vec<Command>>,
    code: Option<i32>,
}
impl Runner for Runs {
    fn run(&self, command: &Command) -> Result<Option<i32>, String> {
        self.seen.borrow_mut().push(command.clone());
        Ok(self.code)
    }
}

fn good() -> Fetch {
    Fetch { name: "cadcraft-1.2.3-windows-x64.msi", bytes: b"installer bytes", sum: None, outside: false, fail: false }
}
fn signed_by(org: &str) -> Signer {
    Signer(SignatureCheck::Valid { organization: Some(org.into()) })
}

/// Runs `request` through an engine; returns the result and how many times msiexec was started.
fn run(request: &[u8], fetch: &Fetch, signer: &Signer, code: Option<i32>) -> (Result<Outcome, Refusal>, usize) {
    run_on(request, &Mach(Some("1.2.2")), fetch, signer, code)
}

fn run_on(request: &[u8], machine: &dyn Machine, fetch: &Fetch, signer: &Signer, code: Option<i32>) -> (Result<Outcome, Refusal>, usize) {
    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    let runner = Runs { code, ..Runs::default() };
    let roots = [pf_root()];
    let engine = Engine {
        staging_root: &staging,
        program_files: &roots,
        msiexec: Path::new("msiexec.exe"),
        machine,
        fetcher: fetch,
        signatures: signer,
        runner: &runner,
    };
    let result = engine.handle(request);
    let started = runner.seen.borrow().len();
    (result, started)
}

#[test]
fn only_updates_what_is_installed_per_machine_and_only_forward() {
    let (fetch, signer) = (good(), signed_by(PUBLISHER));
    let (result, started) = run_on(REQUEST, &Mach(None), &fetch, &signer, Some(0));
    assert_eq!((result, started), (Err(Refusal::NotInstalled), 0));
    for installed in ["1.2.3", "1.2.4", "2.0.0"] {
        let (result, started) = run_on(REQUEST, &Mach(Some(installed)), &fetch, &signer, Some(0));
        assert_eq!((result, started), (Err(Refusal::NotNewer), 0), "installed {installed}");
    }
    // A pre-release recorded as its plain numbers counts as current, as in the launcher.
    let (result, _) = run_on(br#"{"app":"cadcraft","version":"1.2.3-rc.1"}"#, &Mach(Some("1.2.3")), &fetch, &signer, Some(0));
    assert_eq!(result, Err(Refusal::NotNewer));
    for installed in ["1.2.2", "1.0.0", "0.9.9", "1.2.3-rc.1"] {
        assert_eq!(run_on(REQUEST, &Mach(Some(installed)), &fetch, &signer, Some(0)).0, Ok(Outcome::Installed), "installed {installed}");
    }
}

#[test]
fn installs_a_good_signed_package() {
    let (result, started) = run(REQUEST, &good(), &signed_by(PUBLISHER), Some(0));
    assert_eq!(result, Ok(Outcome::Installed));
    assert_eq!(started, 1);
}

#[test]
fn passes_on_what_the_installer_said() {
    assert_eq!(run(REQUEST, &good(), &signed_by(PUBLISHER), Some(3010)).0, Ok(Outcome::InstalledRestartNeeded));
    assert_eq!(run(REQUEST, &good(), &signed_by(PUBLISHER), Some(1618)).0, Ok(Outcome::Busy));
    assert_eq!(run(REQUEST, &good(), &signed_by(PUBLISHER), Some(1603)).0, Ok(Outcome::Failed(1603)));
}

// Windows only: the install folder is a Windows path (backslashes).
#[cfg(windows)]
#[test]
fn the_command_names_only_the_staged_file() {
    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    let runner = Runs { code: Some(0), ..Runs::default() };
    let (fetch, signer) = (good(), signed_by(PUBLISHER));
    let roots = [pf_root()];
    let engine = Engine {
        staging_root: &staging,
        program_files: &roots,
        msiexec: Path::new("msiexec.exe"),
        machine: &Mach(Some("1.2.2")),
        fetcher: &fetch,
        signatures: &signer,
        runner: &runner,
    };
    engine.handle(REQUEST).unwrap();
    let seen = runner.seen.borrow();
    let args: Vec<String> = seen[0].args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
    assert_eq!(&args[0], "/i");
    assert!(PathBuf::from(&args[1]).ends_with("cadcraft-1.2.3-windows-x64.msi"));
    assert_eq!(&args[2..], ["/qn", "/norestart", "/l*v", &args[5], "ALLUSERS=1"]);
    // And the app stays in the folder Windows Installer says it is in.
    assert_eq!(seen[0].raw_args.len(), 1);
    assert!(seen[0].raw_args[0].starts_with("INSTALLFOLDER=\"") && seen[0].raw_args[0].ends_with("ArtCraft\\CADCraft\""), "{:?}", seen[0].raw_args);
}

#[test]
fn nothing_runs_on_a_bad_request() {
    for body in [&b"{}"[..], br#"{"app":"artcraft-launcher","version":"1.0.0"}"#, br#"{"app":"cadcraft","version":"1.2.3","path":"x"}"#] {
        let (result, started) = run(body, &good(), &signed_by(PUBLISHER), Some(0));
        assert!(result.is_err());
        assert_eq!(started, 0);
    }
}

#[test]
fn nothing_runs_when_the_fetch_fails() {
    let fetch = Fetch { fail: true, ..good() };
    let (result, started) = run(REQUEST, &fetch, &signed_by(PUBLISHER), Some(0));
    assert_eq!(result, Err(Refusal::Fetch("offline".into())));
    assert_eq!(started, 0);
}

#[test]
fn nothing_runs_when_the_file_is_outside_staging() {
    let fetch = Fetch { outside: true, ..good() };
    let (result, started) = run(REQUEST, &fetch, &signed_by(PUBLISHER), Some(0));
    assert_eq!(result, Err(Refusal::NotStaged));
    assert_eq!(started, 0);
}

#[test]
fn nothing_runs_when_the_checksum_differs() {
    for sum in ["0".repeat(64), String::new(), "not a checksum".into()] {
        let fetch = Fetch { sum: Some(sum), ..good() };
        let (result, started) = run(REQUEST, &fetch, &signed_by(PUBLISHER), Some(0));
        assert_eq!(result, Err(Refusal::HashMismatch));
        assert_eq!(started, 0);
    }
}

#[test]
fn nothing_runs_for_the_wrong_package() {
    for name in [
        "deckcraft-1.2.3-windows-x64.msi",
        "cadcraft-1.2.2-windows-x64.msi",
        "cadcraft-1.2.3-windows-x64-portable.zip",
        "cadcraft-1.2.3-linux-x64.msi",
        "cadcraft-1.2.3-windows-x64.exe",
        "cadcraft-1.2.3-windows-mips.msi",
        "evil-cadcraft-1.2.3-windows-x64.msi",
    ] {
        let fetch = Fetch { name, ..good() };
        let (result, started) = run(REQUEST, &fetch, &signed_by(PUBLISHER), Some(0));
        assert_eq!(result, Err(Refusal::WrongPackage), "{name}");
        assert_eq!(started, 0);
    }
}

#[test]
fn every_architecture_of_the_right_package_is_accepted() {
    for arch in ["x64", "x86", "arm64"] {
        let name: &'static str = Box::leak(format!("cadcraft-1.2.3-windows-{arch}.msi").into_boxed_str());
        let (result, _) = run(REQUEST, &Fetch { name, ..good() }, &signed_by(PUBLISHER), Some(0));
        assert_eq!(result, Ok(Outcome::Installed), "{arch}");
    }
}

#[test]
fn nothing_runs_when_unsigned_or_signed_by_someone_else() {
    let (result, started) = run(REQUEST, &good(), &Signer(SignatureCheck::Invalid("no signature".into())), Some(0));
    assert!(matches!(result, Err(Refusal::Unsigned(_))));
    assert_eq!(started, 0);

    for org in ["Microsoft Corporation", "Learning Machines LLC", "Learning Machines Inc."] {
        let (result, started) = run(REQUEST, &good(), &signed_by(org), Some(0));
        assert!(matches!(result, Err(Refusal::WrongPublisher(_))), "{org}");
        assert_eq!(started, 0);
    }
}

#[test]
fn declines_instead_of_failing_when_the_folder_is_unknown_or_outside_program_files() {
    let (fetch, signer) = (good(), signed_by(PUBLISHER));
    let outside = tempfile::tempdir().unwrap();
    let cases: [(&str, MachAt); 3] = [
        ("unknown folder", MachAt(None)),
        ("outside Program Files", MachAt(Some(outside.path().to_path_buf()))),
        ("missing folder", MachAt(Some(pf_root().join("ArtCraft").join("NoSuchApp")))),
    ];
    for (label, machine) in cases {
        let (result, started) = run_on(REQUEST, &machine, &fetch, &signer, Some(0));
        assert!(matches!(&result, Err(r) if r.is_decline()), "{label}: {result:?}");
        assert_eq!(started, 0, "{label}");
    }
}

#[test]
fn a_decline_is_not_a_refusal_of_something_unsafe() {
    assert!(Refusal::Declined("x".into()).is_decline());
    for refusal in [Refusal::NotInstalled, Refusal::NotNewer, Refusal::HashMismatch, Refusal::WrongPackage, Refusal::UnknownApp] {
        assert!(!refusal.is_decline(), "{refusal:?}");
    }
}

// Windows only: it makes an NTFS junction with `mklink /J`.
#[cfg(windows)]
#[test]
fn a_folder_that_is_a_link_out_of_program_files_is_declined() {
    // A junction inside the stand-in Program Files that points outside it: resolved, it is outside.
    let outside = tempfile::tempdir().unwrap();
    let link = pf_root().join("ArtCraft").join(format!("link-{}", std::process::id()));
    let _ = std::fs::remove_dir(&link);
    let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(outside.path()).output().unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stdout));
    let (fetch, signer) = (good(), signed_by(PUBLISHER));
    let (result, started) = run_on(REQUEST, &MachAt(Some(link.clone())), &fetch, &signer, Some(0));
    let _ = std::fs::remove_dir(&link);
    assert!(matches!(&result, Err(r) if r.is_decline()), "{result:?}");
    assert_eq!(started, 0);
}
