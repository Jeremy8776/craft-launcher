//! One Windows permission prompt for several all-users updates.
//!
//! Updating an app installed for all users makes Windows ask for permission, so "Update all" used
//! to ask once per app. Now those jobs wait for each other here, after each has downloaded and
//! checked its own package. When the last has arrived, one elevated PowerShell runs every
//! `msiexec` in turn: one prompt for the whole batch.
//!
//! What the elevated session is given: the fixed `msiexec` command lines of packages the launcher
//! has already downloaded and checked against their release checksums, and nothing else. The
//! launcher's own program is never started elevated with arguments, so nobody can use the prompt
//! to ask it to install something. This is the same trust as the one-app route (a checked package
//! that `msiexec` installs after Windows' prompt); it adds no check and takes none away.
//!
//! A batch of one app is not run here: it keeps the route it always had, where `msiexec` asks for
//! permission itself and the prompt names the package's publisher. If PowerShell cannot start,
//! each package is run the same way, one prompt each, so nobody is left without an update.

#![cfg_attr(not(windows), allow(dead_code))]

use std::cell::Cell;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use artcraft_launcher_core::protocol::CANCELLED;

use super::MsiScope;

/// The queue command that groups all-users jobs into one call.
pub(super) const COMMAND: &str = "windows-installer-all-users";

/// What PowerShell exits with when the permission prompt is declined (`ERROR_CANCELLED`).
const DECLINED: i32 = 1223;

/// Longest encoded script handed to PowerShell. The command line limit is 32,767 characters and
/// the script travels twice (outer and elevated); thirteen apps need about a third of this.
const MAX_ENCODED: usize = 12_000;

/// How many all-users jobs are still preparing their package. The batch starts when it is zero.
pub(super) struct Gather(AtomicUsize);

impl Gather {
    const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    fn preparing(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

static GATHER: Gather = Gather::new();

/// Whether every all-users job that was going to join has joined, or dropped out.
pub(super) fn everyone_here() -> bool {
    GATHER.preparing() == 0
}

/// A job that may be part of a batch. It counts as "still preparing" until it is told it has
/// arrived (its package is ready and it is waiting its turn), or until it is dropped: a job that
/// fails, is cancelled or turns out not to be all-users must never hold the others up.
pub(super) struct Joined<'a> {
    gather: &'a Gather,
    counting: Cell<bool>,
}

impl<'a> Joined<'a> {
    fn on(gather: &'a Gather) -> Self {
        gather.0.fetch_add(1, Ordering::SeqCst);
        Self { gather, counting: Cell::new(true) }
    }

    /// No longer preparing. Safe to call more than once: only the first call counts down.
    pub(super) fn arrived(&self) {
        if self.counting.replace(false) {
            self.gather.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Joined<'static> {
    pub(super) fn join() -> Self {
        Self::on(&GATHER)
    }
}

impl Drop for Joined<'_> {
    fn drop(&mut self) {
        self.arrived();
    }
}

/// `path` as text that can sit inside quotes on a command line.
fn text(path: &Path) -> Result<&str, String> {
    let text = path.to_str().ok_or("unusual characters in a path")?;
    if text.chars().any(|c| c == '"' || c.is_control()) {
        return Err(format!("couldn't use {text}: the path contains invalid characters"));
    }
    Ok(text)
}

/// The `msiexec` arguments for one package, the same ones `msi_command` and the folder and scope
/// arguments give the single-app route, as one command line.
pub(super) fn line(msi: &Path, destination: Option<&Path>, scope: Option<MsiScope>) -> Result<String, String> {
    let msi = super::msi_path(msi)?;
    let log = super::msi_log(&msi);
    let mut line = format!("/i \"{}\" /passive /norestart /l*v \"{}\"", text(&msi)?, text(&log)?);
    if let Some(folder) = destination {
        line.push(' ');
        line.push_str(&super::msi_folder_argument(folder)?);
    }
    for argument in super::msi_scope_arguments(scope) {
        line.push(' ');
        line.push_str(argument);
    }
    Ok(line)
}

/// `text` as a PowerShell single-quoted string. PowerShell also reads the typographic single
/// quotes as quotes, so those are doubled too.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for c in text.chars() {
        out.push(c);
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// What the elevated session runs: each command line through `program` in turn, whatever the
/// others do, and the first failing exit code (0, 3010 and 1641 are success) as its own.
fn inner_script(program: &str, lines: &[String]) -> String {
    let list = lines.iter().map(|l| quoted(l)).collect::<Vec<_>>().join(",");
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         $ProgressPreference = 'SilentlyContinue'\n\
         $program = {}\n\
         $worst = 0\n\
         foreach ($arguments in @({list})) {{\n\
         \x20 $process = Start-Process -FilePath $program -ArgumentList $arguments -Wait -PassThru\n\
         \x20 $code = $process.ExitCode\n\
         \x20 if ($worst -eq 0 -and $code -ne 0 -and $code -ne 3010 -and $code -ne 1641) {{ $worst = $code }}\n\
         }}\n\
         exit $worst\n",
        quoted(program)
    )
}

/// What the launcher runs without elevation: it starts the elevated session (the one prompt) and
/// waits for it. Declining the prompt makes PowerShell throw, which becomes [`DECLINED`].
fn outer_script(powershell: &str, encoded: &str) -> String {
    format!(
        "try {{ $elevated = Start-Process -FilePath {} -Verb RunAs -WindowStyle Hidden -Wait -PassThru \
         -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','{encoded}'; exit $elevated.ExitCode }} \
         catch {{ exit {DECLINED} }}",
        quoted(powershell)
    )
}

/// The base64 form of `script` that `-EncodedCommand` takes: UTF-16 little-endian, so no quoting
/// can go wrong however odd a path is.
fn encoded(script: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let byte = |at: usize| u32::from(chunk.get(at).copied().unwrap_or(0));
        let bits = (byte(0) << 16) | (byte(1) << 8) | byte(2);
        for (position, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if position > chunk.len() {
                out.push('=');
            } else if let Some(letter) = usize::try_from((bits >> shift) & 63).ok().and_then(|index| ALPHABET.get(index)) {
                out.push(char::from(*letter));
            }
        }
    }
    out
}

/// What a batch's exit code means for each of its `count` apps. A failure of a shared call can't
/// be pinned on one app; the backend then looks at what is installed now (`landed_despite`).
pub(super) fn outcome(code: Option<i32>, count: usize) -> Result<(), String> {
    let result = match code {
        Some(DECLINED) => Err(CANCELLED.to_string()),
        other => super::explain(other),
    };
    match result {
        Err(e) if count > 1 && e != CANCELLED => Err(format!("{e} while handling {count} {}", super::SHARED_CALL)),
        other => other,
    }
}

/// Runs every package in `targets` (command lines from [`line`]) under one permission prompt.
#[cfg(windows)]
pub(super) fn run(msiexec: &Path, targets: &[std::ffi::OsString]) -> Result<(), String> {
    use std::path::PathBuf;

    let lines: Vec<String> =
        targets.iter().map(|t| t.to_str().map(str::to_string).ok_or("unusual characters in a path")).collect::<Result<_, _>>()?;
    let root =
        std::env::var_os("SystemRoot").map(PathBuf::from).filter(|root| root.is_absolute()).ok_or("SystemRoot is not set to an absolute path")?;
    let powershell = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let script = encoded(&inner_script(text(msiexec)?, &lines));
    if script.len() > MAX_ENCODED {
        return Err(format!("too many apps to update in one go ({}). Try them one at a time.", lines.len()));
    }
    let mut command = crate::process::command(&powershell);
    command.args(["-NoProfile", "-NonInteractive", "-Command"]).arg(outer_script(text(&powershell)?, &script));
    log::info!("updating {} all-users apps under one permission prompt", lines.len());
    match command.status() {
        Ok(status) => outcome(status.code(), lines.len()),
        Err(e) => {
            log::warn!("couldn't start PowerShell ({e}); updating each app with its own prompt");
            each_alone(msiexec, &lines)
        }
    }
}

/// The fallback: every package through `msiexec` by itself, as the single-app route does.
#[cfg(windows)]
fn each_alone(msiexec: &Path, lines: &[String]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let mut first_failure = None;
    for line in lines {
        let status = crate::process::command(msiexec).raw_arg(line).status().map_err(|e| format!("couldn't start the Windows Installer ({e})"))?;
        if first_failure.is_none() && super::explain(status.code()).is_err() {
            first_failure = Some(status.code());
        }
    }
    outcome(first_failure.unwrap_or(Some(0)), lines.len())
}

#[cfg(test)]
mod tests;
