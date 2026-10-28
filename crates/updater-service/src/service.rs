//! The Windows service itself: startup checks, the real engine, and the control handler.
//!
//! Registered once by an installer (one UAC prompt) to run as SYSTEM:
//! `sc create ArtCraftUpdater binPath= "<Program Files>\ArtCraft\Updater\artcraft-updater-service.exe run"`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use crate::engine::Engine;
use crate::fetch::GitHubFetcher;
use crate::install::SystemRunner;
use crate::journal::Journal;
use crate::machine::Registry;
use crate::pipe::{self, AnyCaller, Config};
use crate::request;
use crate::wintrust::WinTrust;

pub const SERVICE_NAME: &str = "ArtCraftUpdater";

define_windows_service!(ffi_service_main, service_main);

/// Hands this process to the Service Control Manager. Returns when the service stops.
pub fn run_dispatcher() -> windows_service::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

fn service_main(_arguments: Vec<OsString>) {
    // The control manager has no one to show an error to; `run` writes it to the journal.
    let _ = run();
}

fn run() -> Result<(), String> {
    let stop = Arc::new(AtomicBool::new(false));
    let handler_stop = Arc::clone(&stop);
    let status = service_control_handler::register(SERVICE_NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            handler_stop.store(true, Ordering::SeqCst);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })
    .map_err(|e| format!("couldn't register with the service manager ({e})"))?;

    let report = |state: ServiceState, accept: ServiceControlAccept, code: u32| {
        let _ = status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(code),
            checkpoint: 0,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        });
    };

    report(ServiceState::StartPending, ServiceControlAccept::empty(), 0);
    let outcome = start(&stop, || report(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN, 0));
    report(ServiceState::Stopped, ServiceControlAccept::empty(), u32::from(outcome.is_err()));
    outcome
}

/// Checks where we are, builds the real engine and serves until `stop`. `running` is called once
/// the checks pass, just before the first request can arrive.
fn start(stop: &AtomicBool, running: impl FnOnce()) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("couldn't find the service program ({e})"))?;
    let dir = exe.parent().ok_or("the service program has no folder")?.to_path_buf();
    let journal = Journal::open(&dir.join("updater.log"));
    let result = serve(&exe, &journal, stop, running);
    match &result {
        Ok(()) => journal.line("stopped"),
        Err(why) => journal.line(&format!("could not run: {why}")),
    }
    result
}

fn serve(exe: &Path, journal: &Journal, stop: &AtomicBool, running: impl FnOnce()) -> Result<(), String> {
    let staging = staging_dir(exe, &program_files())?;
    let msiexec = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from).join(r"System32\msiexec.exe");
    let (fetcher, trust) = (GitHubFetcher::new()?, WinTrust::new()?);
    let engine =
        Engine { staging_root: &staging, msiexec: &msiexec, machine: &Registry, fetcher: &fetcher, signatures: &trust, runner: &SystemRunner };
    clean(&staging);
    journal.line(&format!("started, staging in {}", staging.display()));
    running();

    let handler = |body: &[u8]| {
        let asked = request::parse(body).map_or_else(|_| "an unreadable request".to_string(), |r| format!("{} {}", r.app.id, r.version));
        journal.line(&format!("asked to update {asked}"));
        let result = engine.handle(body);
        match &result {
            Ok(outcome) => journal.line(&format!("{asked}: {outcome:?}")),
            Err(refusal) => journal.line(&format!("{asked}: refused, {refusal}")),
        }
        clean(&staging);
        result
    };
    pipe::serve(&Config::default(), &handler, &AnyCaller, stop).map_err(|e| format!("the pipe stopped ({e})"))
}

/// Removes old downloads from staging: a finished package only stays for an hour.
fn clean(staging: &Path) {
    artcraft_launcher_native::http::clean_downloads(staging, Duration::from_secs(24 * 3600), Duration::from_secs(3600));
}

/// The Program Files folders of this PC.
fn program_files() -> Vec<PathBuf> {
    ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"].iter().filter_map(std::env::var_os).map(PathBuf::from).collect()
}

/// The staging folder, `staging` beside the service program, once it is known to be safe.
///
/// The checks in [`crate::engine`] only mean something if no normal user can change a package
/// between the check and the install. `C:\Program Files` gives that for free: by default only
/// SYSTEM, Administrators and TrustedInstaller can write there, and a new folder inherits it. So
/// the service refuses to run from anywhere else, and refuses a `staging` that is a link.
pub fn staging_dir(exe: &Path, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let exe = dunce::canonicalize(exe).map_err(|e| format!("couldn't resolve the service program ({e})"))?;
    let dir = exe.parent().ok_or("the service program has no folder")?;
    let safe = roots.iter().filter_map(|r| dunce::canonicalize(r).ok()).any(|root| dir != root && dir.starts_with(&root));
    if !safe {
        return Err(format!("the service must run from inside Program Files, not {}", dir.display()));
    }
    let staging = dir.join("staging");
    if let Ok(meta) = std::fs::symlink_metadata(&staging) {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        if meta.file_type().is_symlink() || meta.file_attributes() & REPARSE_POINT != 0 || !meta.is_dir() {
            return Err("the staging folder is a link or not a folder".into());
        }
    }
    std::fs::create_dir_all(&staging).map_err(|e| format!("couldn't create the staging folder ({e})"))?;
    let resolved = dunce::canonicalize(&staging).map_err(|e| format!("couldn't resolve the staging folder ({e})"))?;
    if resolved.parent() != Some(dir) {
        return Err("the staging folder does not resolve to a folder beside the service program".into());
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests;
