//! What the service tells its caller. Every refusal happens before any installer runs.

use std::fmt;

/// The service declined to run an installer. Nothing was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The request was larger than [`crate::request::MAX_REQUEST_BYTES`].
    TooLarge,
    /// The request was not the expected JSON, or carried an unknown field.
    Malformed,
    /// The app is not one of the Craft apps in the catalog.
    UnknownApp,
    /// The version is not a plain version number.
    BadVersion,
    /// The app has no per-machine copy to update. The service updates; it never installs fresh.
    NotInstalled,
    /// The version asked for is not newer than the installed one. No downgrades or reinstalls.
    NotNewer,
    /// The service can't safely update this app by itself. Nothing is wrong: the launcher should
    /// use its normal update route, which asks Windows for permission, instead.
    Declined(String),
    /// The release could not be fetched. The text is for the log, not for parsing.
    Fetch(String),
    /// The package is not a regular file inside the staging folder.
    NotStaged,
    /// The package is not the installer for the app that was asked for.
    WrongPackage,
    /// The package is larger than [`crate::stage::MAX_PACKAGE_BYTES`].
    PackageTooLarge,
    /// The package does not match the checksum the release published.
    HashMismatch,
    /// The package has no valid signature.
    Unsigned(String),
    /// The package is validly signed, but not by us.
    WrongPublisher(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("the request was too large"),
            Self::Malformed => f.write_str("the request was not understood"),
            Self::UnknownApp => f.write_str("that app is not part of the Craft App Suite"),
            Self::BadVersion => f.write_str("that is not a version number"),
            Self::NotInstalled => f.write_str("that app is not installed for all users on this PC"),
            Self::NotNewer => f.write_str("that version is not newer than the one installed"),
            Self::Declined(why) => write!(f, "left to the normal update ({why})"),
            Self::Fetch(why) => write!(f, "couldn't get the update ({why})"),
            Self::NotStaged => f.write_str("the update was not where the service keeps its downloads"),
            Self::WrongPackage => f.write_str("the update is not the installer for that app"),
            Self::PackageTooLarge => f.write_str("the update is larger than any Craft App installer"),
            Self::HashMismatch => f.write_str("the update does not match its published checksum"),
            Self::Unsigned(why) => write!(f, "the update is not validly signed ({why})"),
            Self::WrongPublisher(who) => write!(f, "the update is signed by {who}, not by the Craft App publisher"),
        }
    }
}

impl Refusal {
    /// Whether this is a polite "not me" (the launcher falls back) and not a refusal of something unsafe.
    pub fn is_decline(&self) -> bool {
        matches!(self, Self::Declined(_))
    }
}

impl std::error::Error for Refusal {}

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
