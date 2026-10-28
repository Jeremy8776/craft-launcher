//! Engine of the ArtCraft updater service (spike).
//!
//! Purpose: let per-machine Craft App installs update without a UAC prompt for every update.
//! A Windows service running as SYSTEM is installed once (one UAC prompt) and then installs
//! updates on behalf of the launcher. Because that service is a privilege-escalation target,
//! this crate is deliberately small, pure where it can be, and refuses by default.
//!
//! Invariants (each has a test, and `mutation-check.py` breaks each one to prove it):
//!
//! 1. The caller names an app and a version. It never names a path, an URL or an installer
//!    argument. The worst a local process can ask for is an official, signed release.
//! 2. Only the 13 apps in the launcher's catalog are accepted, not the launcher itself.
//! 3. The package must sit inside the service's own staging folder, after resolving links. The
//!    folder is under Program Files and inherits its ACL (by default only SYSTEM, Administrators
//!    and TrustedInstaller write there). The service relies on that default and does not read it;
//!    see `stage.rs`.
//! 4. Its SHA-256 must equal the release's `SHA256SUMS.txt` entry.
//! 5. Its Authenticode signature must be valid and the signer's organisation must be exactly
//!    one of [`publisher::PUBLISHER_ORGANISATIONS`].
//! 6. The installer command line is built here from fixed text. Nothing from the request is
//!    copied into it except the staged file's path.
//! 7. The installer's own `ProductName`, `Manufacturer` and `ProductVersion` must be the
//!    requested app's at the requested version ([`package`]). The file name, checksum and
//!    signature do not say which product an installer installs; this does.
//!
//! Not in this crate yet: the service shell, the authenticated pipe, the real release fetcher
//! (it reuses `artcraft-launcher-native`'s HTTP and `SHA256SUMS` code) and the MSI that
//! registers the service. See `README.md` in this folder.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod engine;
pub mod error;
#[cfg(windows)]
pub mod fetch;
pub mod install;
pub mod journal;
pub mod machine;
pub mod package;
#[cfg(windows)]
pub mod pipe;
pub mod publisher;
pub mod request;
#[cfg(windows)]
pub mod service;
pub mod stage;
#[cfg(windows)]
pub mod wintrust;
pub mod wire;

pub use engine::{Engine, Fetched, Fetcher};
pub use error::{Outcome, Refusal};
