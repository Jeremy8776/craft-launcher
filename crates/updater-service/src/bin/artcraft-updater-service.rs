//! The updater service program. Registered once (one UAC prompt) to run as SYSTEM; see
//! `service.rs` for the checks it makes before it serves.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("run") => match artcraft_launcher_updater_service::service::run_dispatcher() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("This program is started by Windows as a service ({e}).");
                std::process::ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("artcraft-updater-service run   (started by the Windows service manager)");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("The updater service is for Windows only.");
    std::process::ExitCode::from(2)
}
