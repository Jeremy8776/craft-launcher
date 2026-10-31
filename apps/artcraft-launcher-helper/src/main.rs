//! ArtCraft Launcher's installer helper: the only program of the launcher that runs elevated.
//!
//! The launcher starts it behind one permission prompt for all-users Windows Installer work
//! (`artcraft_launcher_native::install::elevated`). It is a separate program, not the launcher
//! started again, so that what runs elevated loads no DLL from its own folder, which the person's
//! programs can write (the launcher installs per user):
//! - its own imports come only from System32 (`/DEPENDENTLOADFLAG:0x800`, `build.rs`), and it
//!   imports no graphics or windowing code, whose system DLLs would look in this folder for their
//!   own imports before `main` could stop them;
//! - the first thing `main` does is limit every later load to System32
//!   ([`load_from_system32_only`]), which covers DLLs that system DLLs load on demand, such as the
//!   `msasn1.dll` crypt32 loads during the signature check. If that fails, it does nothing else.
//!
//! It opens no window. Its log goes into the run folder, which only administrators can change.

#![cfg_attr(windows, windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod run_log;

use std::process::ExitCode;

fn main() -> ExitCode {
    #[cfg(windows)]
    {
        if !load_from_system32_only() {
            return ExitCode::FAILURE;
        }
        let log = run_log::install();
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let done = artcraft_launcher_native::install::elevated::finish(&args, |dir| {
            if let Some(log) = &log {
                log.attach(dir);
            }
        });
        if matches!(done, Some(Ok(()))) { ExitCode::SUCCESS } else { ExitCode::FAILURE }
    }
    // Only Windows has all-users installs to manage.
    #[cfg(not(windows))]
    ExitCode::FAILURE
}

/// Makes every later DLL load in this process search System32 only, never this program's folder,
/// the current folder or `PATH`. Whether Windows accepted it.
///
/// The one `unsafe` in the launcher, allowed here and nowhere else: winsafe doesn't wrap
/// `SetDefaultDllDirectories`, and without it a DLL planted beside this program would be loaded
/// into an elevated process.
#[cfg(windows)]
#[allow(unsafe_code)]
fn load_from_system32_only() -> bool {
    use windows_sys::Win32::System::LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories};
    // SAFETY: the call takes one flag by value and touches no memory of ours. It has no
    // precondition, and failure is reported through its return value, which is checked.
    unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) != 0 }
}
