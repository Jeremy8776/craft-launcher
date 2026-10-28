//! The order of checks. Anything that fails stops here, before an installer runs.

use std::path::{Path, PathBuf};

use artcraft_launcher_core::CraftApp;
use artcraft_launcher_core::release::valid_file_name;
use artcraft_launcher_core::version;

use crate::error::{Outcome, Refusal};
use crate::install::{Runner, interpret, msi_command};
use crate::machine::Machine;
use crate::package::{Packages, require_identity};
use crate::publisher::{Signatures, require_publisher};
use crate::request::{self, InstallRequest};
use crate::stage::{dir_inside, locate, same_checksum, sha256_hex};

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
    /// The Program Files folders. An app is updated only if its recorded folder is inside one.
    pub program_files: &'a [PathBuf],
    pub msiexec: &'a Path,
    pub machine: &'a dyn Machine,
    pub fetcher: &'a dyn Fetcher,
    pub signatures: &'a dyn Signatures,
    /// Reads the staged installer's own product name and version. Called only after the signature
    /// has been accepted, so only a file our publisher signed is ever parsed.
    pub packages: &'a dyn Packages,
    pub runner: &'a dyn Runner,
}

impl Engine<'_> {
    /// Handles one request from the launcher. `Err` means nothing was installed.
    pub fn handle(&self, request_bytes: &[u8]) -> Result<Outcome, Refusal> {
        let request = request::parse(request_bytes)?;
        // An update, never a fresh install, and never to the same or an older version.
        let installed = self.machine.installed(request.app).ok_or(Refusal::NotInstalled)?;
        if !version::is_update(&request.version, &installed.version) {
            return Err(Refusal::NotNewer);
        }
        // Where the installed copy is comes from Windows Installer's admin-only machine records. A
        // copy outside Program Files may sit in a folder a normal user controls, and SYSTEM must not
        // write into one of those (a link swapped in after the check would send the files anywhere),
        // so it is left to the launcher's own route, which asks Windows for permission itself.
        let folder = installed.folder.as_deref().ok_or_else(|| Refusal::Declined("it can't be confirmed where this app is installed".into()))?;
        if !self.program_files.iter().any(|root| dir_inside(root, folder)) {
            return Err(Refusal::Declined("this app is installed outside Program Files".into()));
        }
        let fetched = self.fetcher.fetch(request.app, &request.version, self.staging_root).map_err(Refusal::Fetch)?;

        let staged = locate(self.staging_root, &fetched.file)?;
        require_package_for(&request, &staged)?;
        if !same_checksum(&sha256_hex(&staged)?, &fetched.expected_sha256) {
            return Err(Refusal::HashMismatch);
        }
        require_publisher(self.signatures.check(&staged))?;
        // The name, checksum and signature say who made the file, not which product it installs.
        // A different Craft App's installer, signed by us and renamed, would pass all three.
        let inside = self.packages.read(&staged).map_err(|_| Refusal::WrongPackage)?;
        require_identity(request.app, &request.version, &inside)?;

        let command = msi_command(self.msiexec, &staged, Some(folder))?;
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
