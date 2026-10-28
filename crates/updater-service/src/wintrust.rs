//! The Windows implementation of [`crate::publisher::Signatures`]: Authenticode through WinTrust.
//!
//! `verifysign` is a safe wrapper (the workspace forbids `unsafe`) already used by the
//! launcher's native crate. It asks Windows to verify the signature and the certificate chain,
//! and for an MSI or an exe it reports the signing certificate's subject.

use std::path::Path;

use crate::publisher::{SignatureCheck, Signatures};

/// Verifies with the operating system's default policy.
pub struct WinTrust;

impl Signatures for WinTrust {
    fn check(&self, file: &Path) -> SignatureCheck {
        match verifysign::verify_file(file) {
            Ok(signature) => SignatureCheck::Valid { organization: signature.subject_name().organization },
            Err(error) => SignatureCheck::Invalid(error.to_string()),
        }
    }
}
