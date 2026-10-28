//! How an installer run ended. Shared by the service (which reports it) and the launcher (which
//! reads it).

/// The installer ran. This is what came of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Installed.
    Installed,
    /// Installed; Windows wants a restart before the new files take over.
    InstalledRestartNeeded,
    /// Another install is running. Try again later.
    Busy,
    /// The user cancelled, or Windows Installer was interrupted.
    Cancelled,
    /// The installer stopped with this Windows Installer error code.
    Failed(i32),
}
