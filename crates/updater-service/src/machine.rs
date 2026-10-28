//! What is installed for all users on this PC. The service only updates what is already there.

use artcraft_launcher_core::CraftApp;

/// The version of `app` installed per-machine, if any.
pub trait Machine {
    fn installed(&self, app: &CraftApp) -> Option<semver::Version>;
}

/// Reads Windows Installer's own records through the launcher's detection code, so the service
/// and the launcher can never disagree about what "installed" means. The service runs as SYSTEM,
/// whose per-user records are empty, so only the machine's records can match; the scope filter
/// states that out loud anyway.
#[cfg(windows)]
pub struct Registry;

#[cfg(windows)]
impl Machine for Registry {
    fn installed(&self, app: &CraftApp) -> Option<semver::Version> {
        use artcraft_launcher_core::version::compare;
        use artcraft_launcher_native::detect::windows::msi_installs;
        use artcraft_launcher_native::install::MsiScope;

        msi_installs(app).into_iter().filter(|m| m.scope == MsiScope::PerMachine).filter_map(|m| m.version).max_by(compare)
    }
}
