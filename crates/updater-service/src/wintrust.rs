//! The Windows implementation of [`crate::publisher::Signatures`]: Authenticode through WinTrust.
//!
//! `verifysign` is a safe wrapper (the workspace forbids `unsafe`) already used by the
//! launcher's native crate. It asks Windows to verify the signature and the certificate chain,
//! and for an MSI or an exe it reports the signing certificate's subject.
//!
//! Unlike `verifysign`'s default, this checks the signing certificate for revocation, over the
//! network. A SYSTEM service should not install something signed with a certificate that has been
//! revoked, and it only runs when the PC is online anyway (it has just downloaded the package). If
//! the revocation check can't be done, the file is refused.

use std::path::Path;

use verifysign::{CodeSignVerifier, Revocation, VerifyPolicy};

use crate::publisher::{SignatureCheck, Signatures};

pub struct WinTrust(CodeSignVerifier);

impl WinTrust {
    pub fn new() -> Result<Self, String> {
        let policy = VerifyPolicy::new().revocation(Revocation::EndCertificate).allow_network(true);
        CodeSignVerifier::new(policy).map(Self).map_err(|e| format!("couldn't set up signature checking ({e})"))
    }
}

impl Signatures for WinTrust {
    fn check(&self, file: &Path) -> SignatureCheck {
        match self.0.verify_file(file) {
            Ok(signature) => SignatureCheck::Valid { organization: signature.subject_name().organization },
            Err(error) => SignatureCheck::Invalid(error.to_string()),
        }
    }
}
