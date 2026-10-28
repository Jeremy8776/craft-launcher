//! The launcher's real install path against a real updater service. Ignored by default: it needs a
//! per-machine CADCraft and the real service already running, which `e2e/launcher_path.ps1` sets
//! up (elevated, one UAC prompt) and removes again. Run from a normal, non-elevated process.
//!
//! What is real: Windows Installer's records (detection and who the app is installed for), the
//! launcher's `install` entry point, the pipe, the service, its download from GitHub, and
//! `msiexec`. What is fake: nothing in the update itself. The asset handed to the launcher points
//! at a loopback address that refuses connections, so if the service did NOT do the update, the
//! launcher's own route would fail at its download instead of showing a Windows prompt.
#![cfg(windows)]
#![allow(clippy::unwrap_used)]

use std::time::{Duration, Instant};

use artcraft_launcher_core::protocol::{InstallLocations, JobPhase};
use artcraft_launcher_core::release::{Asset, Release};
use artcraft_launcher_core::{CraftApp, InstallSource, PackageKind, version};
use artcraft_launcher_native::detect;
use artcraft_launcher_native::http::{Http, Stop};
use artcraft_launcher_native::install::{self, Job};
use artcraft_launcher_native::paths::Paths;
use artcraft_launcher_updater_client::Answer;

fn versions() -> (String, String) {
    (std::env::var("E2E_FROM").unwrap_or_default(), std::env::var("E2E_TO").unwrap_or_default())
}

#[test]
#[ignore = "needs the real updater service and a per-machine CADCraft: see e2e/launcher_path.ps1"]
fn an_all_users_app_is_updated_by_the_service_through_the_launchers_install_path() {
    let (from, to) = versions();
    let app = CraftApp::by_id("cadcraft").unwrap();

    // The launcher's own detection sees the app and who it belongs to.
    let existing = detect::windows::detect(app).expect("a per-machine CADCraft must be installed");
    assert_eq!(existing.source, InstallSource::Msi);
    assert_eq!(existing.version, version::parse(&from), "unexpected starting version");

    // The service is up and reachable by a normal user: a request it must refuse (not newer).
    let probe = artcraft_launcher_updater_client::ask("cadcraft", &from);
    assert!(matches!(probe, Answer::Refused(ref why) if why.contains("not newer")), "{probe:?}");

    let release = Release { tag: format!("v{to}"), version: version::parse(&to), ..Release::default() };
    // Refuses connections, and has no `/releases/download/` part, so nothing real can be fetched from it.
    let asset = Asset { name: format!("cadcraft-{to}-windows-x64.msi"), url: "http://127.0.0.1:9/must-not-download.msi".into(), ..Asset::default() };
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::under(root.path());
    let http = Http::new().unwrap();
    let stop = Stop::default();
    let locations = InstallLocations::default();
    let mut phases = Vec::new();
    let mut phase = |p| phases.push(p);
    let mut job = Job { http: &http, paths: &paths, stop: &stop, phase: &mut phase, install_locations: &locations, replace: false };

    let started = Instant::now();
    let result = install::install(app, &release, &asset, PackageKind::Msi, Some(&existing), None, &mut job);
    println!("install result: {result:?} in {:?}", started.elapsed());

    assert_eq!(result, Ok(None), "the launcher's install path did not finish the update");
    assert!(phases.iter().any(|p| matches!(p, JobPhase::Installing)), "{phases:?}");
    assert!(!phases.iter().any(|p| matches!(p, JobPhase::Downloading { .. })), "the launcher downloaded something: {phases:?}");
    assert!(!paths.downloads().exists(), "the launcher made a download folder: the update did not come from the service");

    // And Windows Installer's records say so, as the launcher's own detection reads them.
    let after = detect::windows::detect(app).expect("CADCraft must still be installed");
    assert_eq!(after.version, version::parse(&to), "the app was not updated");
    println!("detected after: {:?} {:?}", after.version, after.source);
}

#[test]
#[ignore = "needs the updater service to be stopped or absent: see e2e/launcher_path.ps1"]
fn with_no_service_the_launcher_is_told_so_at_once_and_leaves_the_update_to_the_normal_route() {
    let started = Instant::now();
    let answer = artcraft_launcher_updater_client::ask("cadcraft", "99.0.0");
    println!("answer without a service: {answer:?} in {:?}", started.elapsed());
    assert!(matches!(answer, Answer::Unavailable(_)), "{answer:?}");
    assert!(!answer.is_installed());
    assert!(started.elapsed() < Duration::from_secs(3), "a missing service must not make the launcher wait");
}
