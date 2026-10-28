//! The order of checks. Anything that fails stops here, before an installer runs.

use std::path::{Path, PathBuf};

use artcraft_launcher_core::CraftApp;
use artcraft_launcher_core::release::valid_file_name;
use artcraft_launcher_core::version;

use crate::error::{Outcome, Refusal};
use crate::install::{Runner, interpret, msi_command};
use crate::machine::Machine;
use crate::publisher::{Signatures, require_publisher};
use crate::request::{self, InstallRequest};
use crate::stage::{locate, same_checksum, sha256_hex};

/// A package the fetcher put in the staging folder, and the checksum its release published.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub file: PathBuf,
    pub expected_sha256: String,
}

/// Gets the release of `app` at `version` from `https://github.com/<repo>/releases/download/`
/// into `staging`, and reads its `SHA256SUMS.txt` entry. The real one will reuse the
/// launcher's HTTP and checksum code, with its download-URL allowlist; tests use a scripted one.
pub trait Fetcher {
    fn fetch(&self, app: &CraftApp, version: &semver::Version, staging: &Path) -> Result<Fetched, String>;
}

/// Everything the engine needs, passed in so tests can replace the outside world.
pub struct Engine<'a> {
    pub staging_root: &'a Path,
    pub msiexec: &'a Path,
    pub machine: &'a dyn Machine,
    pub fetcher: &'a dyn Fetcher,
    pub signatures: &'a dyn Signatures,
    pub runner: &'a dyn Runner,
}

impl Engine<'_> {
    /// Handles one request from the launcher. `Err` means nothing was installed.
    pub fn handle(&self, request_bytes: &[u8]) -> Result<Outcome, Refusal> {
        let request = request::parse(request_bytes)?;
        // An update, never a fresh install, and never to the same or an older version.
        let installed = self.machine.installed(request.app).ok_or(Refusal::NotInstalled)?;
        if !version::is_update(&request.version, &installed) {
            return Err(Refusal::NotNewer);
        }
        let fetched = self.fetcher.fetch(request.app, &request.version, self.staging_root).map_err(Refusal::Fetch)?;

        let staged = locate(self.staging_root, &fetched.file)?;
        require_package_for(&request, &staged)?;
        if !same_checksum(&sha256_hex(&staged)?, &fetched.expected_sha256) {
            return Err(Refusal::HashMismatch);
        }
        require_publisher(self.signatures.check(&staged))?;

        let command = msi_command(self.msiexec, &staged)?;
        let code = self.runner.run(&command).map_err(Refusal::Fetch)?;
        Ok(interpret(code))
    }
}

/// The package's file name must be exactly `<app>-<version>-windows-<arch>.msi` for the app and
/// version asked for. A validly signed installer for a different app, or an older version, is
/// refused: the signature proves who made a file, not that it is the one that was requested.
fn require_package_for(request: &InstallRequest, staged: &Path) -> Result<(), Refusal> {
    let name = staged.file_name().and_then(|n| n.to_str()).ok_or(Refusal::WrongPackage)?;
    if !valid_file_name(name) {
        return Err(Refusal::WrongPackage);
    }
    let accepted =
        request.app.stems().any(|stem| ["x64", "x86", "arm64"].iter().any(|arch| name == format!("{stem}-{}-windows-{arch}.msi", request.version)));
    if accepted { Ok(()) } else { Err(Refusal::WrongPackage) }
}

#[cfg(test)]
mod tests;
