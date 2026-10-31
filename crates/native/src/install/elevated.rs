//! All-users Windows Installer work under one permission prompt, run by the launcher's own program.
//!
//! Going back to an older version of an all-users app takes three `msiexec` steps (remove, install,
//! put back on failure), and asking for permission at each step proved fragile: a refused later
//! prompt can leave no app at all (see [`msi_downgrade`](super::msi_downgrade)). So the launcher
//! starts a copy of its own signed program elevated, once: Windows' prompt names ArtCraft Launcher
//! and its publisher. No script, interpreter or encoded command is involved; every program that
//! runs elevated is ours or `msiexec`, called directly.
//!
//! The elevated copy ([`finish`]) trusts nothing it is handed. It copies each package into a new
//! folder under ProgramData, which ordinary accounts can read but not change, hashing the copy as
//! it goes, and refuses a copy whose SHA-256 is not the release checksum or whose Authenticode
//! publisher is not its own. `msiexec` then installs from that folder, so nothing can be swapped
//! between the check and the install.
//!
//! The prompt gives back no handle to wait on, so the two processes meet in that folder: the
//! elevated copy holds a lock file there without sharing while it works and writes its result
//! beside it before letting go. The launcher waits for the lock to appear, then to be released,
//! then reads the result.

#![cfg_attr(not(windows), allow(dead_code))]

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The first argument of the elevated copy, followed by the run id and the job file.
pub const FLAG: &str = "--elevated-windows-installer";

/// Each run's folder in ProgramData is this followed by the run id.
const RUN_PREFIX: &str = "artcraft-launcher-elevated-";
/// Held without sharing while the elevated copy works.
const LOCK: &str = "running";
/// Written before the lock is let go: `ok`, or `failed` and the message on the next line.
const RESULT: &str = "result";
/// The packages' names in the run folder; `msiexec` writes its log beside each.
const CHOSEN: &str = "chosen.msi";
const PREVIOUS: &str = "previous.msi";
/// Every name a run folder can hold, so an old one is cleared file by file, never recursively.
const RUN_FILES: [&str; 6] = [LOCK, RESULT, CHOSEN, PREVIOUS, "chosen.log", "previous.log"];
/// A job file is a few hundred bytes; anything far larger is not one of ours.
const MAX_JOB: u64 = 64 * 1024;

/// One downloaded package: where it is, the checksum it was checked against, and its version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Package {
    pub file: PathBuf,
    pub sha256: String,
    pub version: String,
}

/// An all-users downgrade: remove `app`'s installed copy, install `chosen` in `folder`, and put
/// `previous` back if that does not finish.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Job {
    pub app: String,
    pub chosen: Package,
    pub previous: Package,
    pub folder: Option<PathBuf>,
}

/// A fresh run id: 32 lower-case hex digits.
fn new_run() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("couldn't prepare the installer helper ({e})"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether `run` has the shape [`new_run`] gives, so it can't name another folder.
fn valid_run(run: &str) -> bool {
    run.len() == 32 && run.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn run_dir(base: &Path, run: &str) -> PathBuf {
    base.join(format!("{RUN_PREFIX}{run}"))
}

/// The elevated copy's command line: [`FLAG`], the run id and the job file in quotes.
fn parameters(run: &str, job: &Path) -> Result<String, String> {
    let job = super::windows_text(job)?;
    if !valid_run(run) || job.ends_with('\\') {
        return Err(format!("couldn't prepare the installer helper for {job}"));
    }
    Ok(format!("{FLAG} {run} \"{job}\""))
}

/// Reads [`parameters`] back. `None` when this program was not started as the elevated copy.
fn parse_args(args: &[OsString]) -> Option<Result<(String, PathBuf), String>> {
    let first = args.first()?;
    if first != FLAG {
        return None;
    }
    Some(match args {
        [_, run, job] if run.to_str().is_some_and(valid_run) => Ok((run.to_string_lossy().into_owned(), PathBuf::from(job))),
        _ => Err("the installer helper's arguments are incomplete".into()),
    })
}

fn read_job(path: &Path) -> Result<Job, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("couldn't read the installer helper's job ({e})"))?;
    let mut text = String::new();
    file.take(MAX_JOB + 1).read_to_string(&mut text).map_err(|e| format!("couldn't read the installer helper's job ({e})"))?;
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_JOB {
        return Err("the installer helper's job is too large".into());
    }
    serde_json::from_str(&text).map_err(|e| format!("couldn't read the installer helper's job ({e})"))
}

/// Copies `package` to a new file `name` in `dir`, hashing what is written, and keeps the copy
/// only when it has the package's checksum. The copy is what gets installed.
fn stage(package: &Package, dir: &Path, name: &str) -> Result<PathBuf, String> {
    let shown = package.file.display();
    let mut source = std::fs::File::open(&package.file).map_err(|e| format!("couldn't read {shown} ({e})"))?;
    let target = dir.join(name);
    let mut copy = std::fs::File::create_new(&target).map_err(|e| format!("couldn't copy {shown} to install it ({e})"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let copied = loop {
        match source.read(&mut buffer) {
            Ok(0) => break copy.flush().map_err(|e| format!("couldn't copy {shown} to install it ({e})")),
            Ok(n) => {
                let chunk = buffer.get(..n).unwrap_or_default();
                hasher.update(chunk);
                if let Err(e) = copy.write_all(chunk) {
                    break Err(format!("couldn't copy {shown} to install it ({e})"));
                }
            }
            Err(e) => break Err(format!("couldn't read {shown} ({e})")),
        }
    };
    drop(copy);
    let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let checked = copied.and_then(|()| {
        if digest.eq_ignore_ascii_case(package.sha256.trim()) {
            Ok(())
        } else {
            Err(format!("{shown} changed after it was checked, so it wasn't installed. Try again."))
        }
    });
    match checked {
        Ok(()) => Ok(target),
        Err(e) => {
            let _ = std::fs::remove_file(&target);
            Err(e)
        }
    }
}

/// Writes the result file. Created new, so nothing already there is followed or overwritten.
fn write_result(dir: &Path, result: &Result<(), String>) -> std::io::Result<()> {
    let text = match result {
        Ok(()) => "ok\n".to_string(),
        Err(e) => format!("failed\n{e}"),
    };
    let mut file = std::fs::File::create_new(dir.join(RESULT))?;
    file.write_all(text.as_bytes())?;
    file.flush()
}

/// What the elevated copy reported. No result once the lock is let go means it stopped part way.
fn read_result(dir: &Path, app: &str) -> Result<(), String> {
    let stopped = || format!("the installer helper stopped unexpectedly. Check which version of {app} is installed.");
    let text = std::fs::read_to_string(dir.join(RESULT)).map_err(|_| stopped())?;
    match text.split_once('\n') {
        Some(("ok", _)) => Ok(()),
        Some(("failed", message)) if !message.trim().is_empty() => Err(message.trim().to_string()),
        _ => Err(stopped()),
    }
}

/// Clears run folders older than a day, left by earlier runs: only the names a run writes, one
/// file at a time, then the folder if that emptied it. Nothing is followed into or recursed.
fn clear_old_runs(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else { return };
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let ours = name.to_str().and_then(|n| n.strip_prefix(RUN_PREFIX)).is_some_and(valid_run);
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > day);
        if ours && old && entry.file_type().is_ok_and(|t| t.is_dir()) {
            let dir = entry.path();
            for file in RUN_FILES {
                let _ = std::fs::remove_file(dir.join(file));
            }
            let _ = std::fs::remove_dir(&dir);
        }
    }
}

/// Whether `file` is signed by the same publisher as `reference` (this program).
#[cfg(windows)]
fn signed_like(reference: &Path, file: &Path) -> Result<(), String> {
    let publisher = |path: &Path| verifysign::verify_file(path).map(|signature| signature.subject_name());
    let ours = publisher(reference).map_err(|e| format!("this copy of ArtCraft Launcher isn't signed ({e}), so it can't install for all users"))?;
    let theirs = publisher(file).map_err(|e| format!("{} isn't signed ({e})", file.display()))?;
    if ours == theirs { Ok(()) } else { Err(format!("{} isn't signed by ArtCraft Launcher's publisher", file.display())) }
}

/// ProgramData, from Windows rather than the environment.
#[cfg(windows)]
fn program_data() -> Result<PathBuf, String> {
    use winsafe::{SHGetKnownFolderPath, co};
    let path = SHGetKnownFolderPath(&co::KNOWNFOLDERID::ProgramData, co::KF::DEFAULT, None)
        .map(PathBuf::from)
        .map_err(|e| format!("couldn't find the ProgramData folder ({e})"))?;
    if path.is_absolute() { Ok(path) } else { Err("couldn't find the ProgramData folder".into()) }
}

/// Where a run's lock stands, as the launcher sees it.
#[derive(Debug, PartialEq, Eq)]
enum Lock {
    /// The elevated copy hasn't started yet.
    Missing,
    Held,
    /// The elevated copy has finished, or stopped.
    Released,
}

#[cfg(windows)]
fn lock_state(lock: &Path) -> Result<Lock, String> {
    /// `ERROR_SHARING_VIOLATION`: the elevated copy holds the file without sharing.
    const SHARING_VIOLATION: i32 = 32;
    match std::fs::File::open(lock) {
        Ok(_) => Ok(Lock::Released),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Lock::Missing),
        Err(e) if e.raw_os_error() == Some(SHARING_VIOLATION) => Ok(Lock::Held),
        Err(e) => Err(format!("couldn't follow the installer helper ({e})")),
    }
}

/// Waits for the elevated copy to take its lock (up to `start`) and then to let it go.
#[cfg(windows)]
fn wait(lock: &Path, start: std::time::Duration) -> Result<(), String> {
    let began = std::time::Instant::now();
    loop {
        match lock_state(lock)? {
            Lock::Released => return Ok(()),
            Lock::Missing if began.elapsed() > start => {
                return Err("permission was given, but the installer helper didn't start, so nothing was changed".into());
            }
            Lock::Missing | Lock::Held => std::thread::sleep(std::time::Duration::from_millis(250)),
        }
    }
}

/// Runs `job` for `app` (its name, for messages) in an elevated copy of this program, behind one
/// permission prompt, and waits for it. A refused prompt is
/// [`CANCELLED`](artcraft_launcher_core::protocol::CANCELLED) and changes nothing.
#[cfg(windows)]
pub(super) fn run(job: &Job, app: &str) -> Result<(), String> {
    use artcraft_launcher_core::protocol::CANCELLED;
    use winsafe::{SHELLEXECUTEINFO, ShellExecuteEx, co};

    let run = new_run()?;
    let downloads = job.chosen.file.parent().ok_or("couldn't find the downloaded package's folder")?;
    let job_file = downloads.join(format!("elevated-{run}.json"));
    let text = serde_json::to_string(job).map_err(|e| format!("couldn't prepare the installer helper ({e})"))?;
    std::fs::File::create_new(&job_file)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|e| format!("couldn't prepare the installer helper ({e})"))?;
    let finished = (|| {
        let me = std::env::current_exe().map_err(|e| format!("couldn't find this program ({e})"))?;
        let parameters = parameters(&run, &job_file)?;
        log::info!("{app}: going back to {} under one permission prompt", job.chosen.version);
        let asked = ShellExecuteEx(&SHELLEXECUTEINFO {
            mask: co::SEE_MASK::NOASYNC | co::SEE_MASK::FLAG_NO_UI,
            verb: Some("runas"),
            file: super::windows_text(&me)?,
            parameters: Some(&parameters),
            // Nothing is hidden: the helper opens no window of its own, and msiexec shows its progress.
            show: co::SW::SHOWNORMAL,
            ..Default::default()
        });
        match asked {
            Ok(()) => {}
            Err(co::ERROR::CANCELLED) => return Err(CANCELLED.to_string()),
            Err(e) => return Err(format!("couldn't ask Windows for permission ({e}), so nothing was changed")),
        }
        let dir = run_dir(&program_data()?, &run);
        wait(&dir.join(LOCK), std::time::Duration::from_secs(60))?;
        read_result(&dir, app)
    })();
    let _ = std::fs::remove_file(&job_file);
    finished
}

/// The elevated copy's work, when this program was started with [`FLAG`]; `args` are its arguments
/// without the program name. `None` otherwise.
#[cfg(windows)]
pub fn finish(args: &[OsString]) -> Option<Result<(), String>> {
    use std::os::windows::fs::OpenOptionsExt;

    let (run, job) = match parse_args(args)? {
        Ok(parsed) => parsed,
        Err(e) => return Some(Err(e)),
    };
    let base = match program_data() {
        Ok(base) => base,
        Err(e) => return Some(Err(e)),
    };
    clear_old_runs(&base);
    let dir = run_dir(&base, &run);
    // A new folder, never one that already exists: its files are then ours alone to change.
    if let Err(e) = std::fs::create_dir(&dir) {
        return Some(Err(format!("couldn't create {} ({e})", dir.display())));
    }
    let lock = match std::fs::File::options().write(true).create_new(true).share_mode(0).open(dir.join(LOCK)) {
        Ok(lock) => lock,
        Err(e) => return Some(Err(format!("couldn't create the installer helper's lock ({e})"))),
    };
    let done = downgrade(&dir, &job);
    if let Err(e) = write_result(&dir, &done) {
        log::warn!("couldn't record the installer helper's result ({e})");
    }
    drop(lock);
    Some(done)
}

/// Checks and stages both packages, then removes, installs and if needed puts back.
#[cfg(windows)]
fn downgrade(dir: &Path, job_file: &Path) -> Result<(), String> {
    use super::{MsiScope, msi_downgrade, windows};

    let job = read_job(job_file)?;
    let app = artcraft_launcher_core::catalog::CraftApp::by_id(&job.app).ok_or_else(|| format!("{} isn't a Craft app", job.app))?;
    let me = std::env::current_exe().map_err(|e| format!("couldn't find this program ({e})"))?;
    let chosen = stage(&job.chosen, dir, CHOSEN)?;
    let previous = stage(&job.previous, dir, PREVIOUS)?;
    signed_like(&me, &chosen)?;
    signed_like(&me, &previous)?;
    let folder = job.folder.as_deref();
    let scope = Some(MsiScope::PerMachine);
    let done = msi_downgrade::run(msi_downgrade::Steps {
        app: app.name,
        chosen: &job.chosen.version,
        previous: &job.previous.version,
        remove: &mut || windows::remove_registrations(app),
        install_chosen: &mut || windows::run_direct(&chosen, folder, scope),
        reinstall_previous: &mut || windows::run_direct(&previous, folder, scope),
    });
    // Windows Installer keeps its own copy of what it installed; the logs stay for diagnosis.
    let _ = std::fs::remove_file(&chosen);
    let _ = std::fs::remove_file(&previous);
    done
}

#[cfg(test)]
#[path = "elevated_tests.rs"]
mod tests;
