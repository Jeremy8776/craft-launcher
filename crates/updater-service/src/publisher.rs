//! Who may have signed a package for the service to install it.

use std::path::Path;

use crate::error::Refusal;

/// The signer organisations the service accepts, compared exactly (not "contains").
///
/// This is the `O=` of the certificate the Craft builds are signed with
/// (`Learning Machines Inc`, Atlanta). Note that the launcher's version resources and install
/// registry keys say `Learning Machines LLC`; that is a different string and is not accepted
/// here. Match what the certificate says, because that is what Windows verified.
///
/// A subject match, not a thumbprint, because Azure Trusted Signing certificates rotate often.
pub const PUBLISHER_ORGANISATIONS: [&str; 1] = ["Learning Machines Inc"];

/// What the operating system said about a file's signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureCheck {
    /// The signature is valid and chains to a trusted root. This is who signed it.
    Valid { organization: Option<String> },
    /// No signature, a broken one, an untrusted chain, or the file was changed after signing.
    Invalid(String),
}

/// Asks the operating system to verify a file's signature. The real one is
/// [`crate::wintrust::WinTrust`]; tests use a scripted one.
pub trait Signatures {
    fn check(&self, file: &Path) -> SignatureCheck;
}

/// Accepts only a valid signature from an accepted publisher.
pub fn require_publisher(check: SignatureCheck) -> Result<(), Refusal> {
    match check {
        SignatureCheck::Invalid(why) => Err(Refusal::Unsigned(why)),
        SignatureCheck::Valid { organization } => match organization {
            Some(org) if PUBLISHER_ORGANISATIONS.contains(&org.as_str()) => Ok(()),
            Some(org) => Err(Refusal::WrongPublisher(org)),
            None => Err(Refusal::WrongPublisher("an unnamed publisher".into())),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(org: Option<&str>) -> SignatureCheck {
        SignatureCheck::Valid { organization: org.map(str::to_string) }
    }

    #[test]
    fn accepts_the_publisher() {
        assert_eq!(require_publisher(valid(Some("Learning Machines Inc"))), Ok(()));
    }

    #[test]
    fn refuses_lookalikes_exactly() {
        for org in [
            "Learning Machines Inc.",
            "learning machines inc",
            "Learning Machines Inc ",
            "Evil Learning Machines Inc",
            "Learning Machines Inc Evil",
            "Learning Machines LLC",
            "Microsoft Corporation",
            "",
        ] {
            assert!(matches!(require_publisher(valid(Some(org))), Err(Refusal::WrongPublisher(_))), "{org:?}");
        }
        assert!(matches!(require_publisher(valid(None)), Err(Refusal::WrongPublisher(_))));
    }

    #[test]
    fn refuses_an_invalid_signature() {
        assert!(matches!(require_publisher(SignatureCheck::Invalid("hash mismatch".into())), Err(Refusal::Unsigned(_))));
    }
}
