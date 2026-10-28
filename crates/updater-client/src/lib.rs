//! The launcher's side of the updater service pipe.
//!
//! Purpose: the service (`artcraft-launcher-updater-service`, a Windows service running as SYSTEM)
//! and the launcher must agree on what goes over the pipe. That agreement lives here, in a crate
//! with no server code, so `artcraft-launcher-native` can ask the service without depending on it
//! (the service depends on `native`, so the other way round would be a cycle).
//!
//! Invariants:
//!
//! 1. A request carries an app id and a version, nothing else ([`request_body`]). The service
//!    refuses a request with any other field, so this crate cannot be used to name a path, an URL
//!    or an installer argument even by mistake.
//! 2. [`ask`] never fails loudly. Every way the service can be missing, slow, wrong or unwilling
//!    comes back as an [`Answer`], and only [`Answer::Installed`] means the update happened. The
//!    caller carries on with its normal update route for every other answer.
//! 3. Nothing here is `unsafe`, and no key, path or personal detail is sent.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod answer;
mod outcome;
#[cfg(windows)]
pub mod pipe;
pub mod wire;

pub use answer::{Answer, WaitError, interpret, interpret_wait, request_body};
#[cfg(windows)]
pub use answer::{ask, ask_cancellable};
pub use outcome::Outcome;
pub use wire::Reply;
