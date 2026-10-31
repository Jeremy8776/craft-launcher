//! A downloaded Windows Installer package, held from its last check until `msiexec` is done with it.
//!
//! A download is checked against its release checksum as it arrives, but it then waits in the
//! person's own downloads folder, which their programs can change. `msiexec` asks for permission
//! itself for an all-users install, so a package swapped after that check would run with
//! administrator rights. So the launcher opens the package so nobody can change, move or delete it
//! (Windows Installer only needs to read it: checked with a real install on Windows 11), hashes it
//! again, checks an all-users package's publisher, and keeps it open until `msiexec` exits.

#![cfg_attr(not(windows), allow(dead_code))]

use std::path::Path;

use artcraft_launcher_core::catalog::{AppKind, CraftApp};

use super::elevated::{hash_through, open_unchangeable};

/// A package nobody can change, move or delete while this is alive.
#[derive(Debug)]
pub(super) struct Held(#[allow(dead_code)] std::fs::File);

/// Holds `file` and checks it still has its release checksum.
pub(super) fn hold(file: &Path, sha256: &str) -> Result<Held, String> {
    let shown = file.display();
    let mut held = open_unchangeable(file).map_err(|e| format!("couldn't read {shown} ({e})"))?;
    let digest = hash_through(&mut held, None).map_err(|e| format!("couldn't read {shown} ({e})"))?;
    if !digest.eq_ignore_ascii_case(sha256.trim()) {
        return Err(format!("{shown} changed after it was downloaded, so it wasn't installed. Try again."));
    }
    Ok(Held(held))
}

/// Whether an install of `app` from `file` must carry this program's publisher: an all-users install
/// of an official app. A community app is signed by its own developer, so its checksum is the check.
fn needs_publisher(app: &CraftApp, all_users: bool) -> bool {
    all_users && app.kind == AppKind::Official
}

/// Checks `file`'s publisher when [`needs_publisher`] says so. A build of the launcher that isn't
/// signed (a local build) has no publisher to compare with, so it only says so in the log.
#[cfg(windows)]
pub(super) fn check_publisher(app: &CraftApp, file: &Path, all_users: bool) -> Result<(), String> {
    if !needs_publisher(app, all_users) {
        return Ok(());
    }
    let me = std::env::current_exe().map_err(|e| format!("couldn't find this program ({e})"))?;
    if let Err(e) = verifysign::verify_file(&me) {
        log::warn!("{}: this copy of ArtCraft Launcher isn't signed ({e}), so the package's publisher wasn't checked", app.name);
        return Ok(());
    }
    super::elevated::signed_like(&me, file)
}

#[cfg(test)]
#[path = "held_tests.rs"]
mod tests;
