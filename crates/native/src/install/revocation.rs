//! Whether a package's signing certificate was withdrawn (revoked), asked before any prompt.
//!
//! The packages carry RFC 3161 timestamps, so a signature made while its certificate was valid
//! stays valid after the certificate expires. Revocation is then the only way the publisher can
//! withdraw a package signed with a stolen or misused certificate. The unelevated launcher asks
//! here, before an all-users install and before a downgrade's prompt, where Windows may fetch the
//! revocation lists. The elevated helper doesn't ask: that would mean network access from an
//! elevated process (storytold/craft-launcher#1080).

#![cfg_attr(not(windows), allow(dead_code))]

/// `CRYPT_E_REVOKED`: a certificate in the chain was revoked.
const REVOKED: i32 = 0x8009_2010_u32.cast_signed();
/// `CRYPT_E_NO_REVOCATION_CHECK`: a certificate names no way to check it.
const NO_CHECK: i32 = 0x8009_2012_u32.cast_signed();
/// `CRYPT_E_REVOCATION_OFFLINE`: the revocation server couldn't be reached.
const OFFLINE: i32 = 0x8009_2013_u32.cast_signed();
/// `CERT_E_REVOCATION_FAILURE`: the check couldn't be completed.
const FAILED: i32 = 0x800B_010E_u32.cast_signed();

/// What a signature check with revocation on found.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Status {
    /// Valid, and no certificate in the chain is revoked.
    Good,
    Revoked,
    /// Valid as far as it went, but nobody could say whether a certificate was revoked.
    Unknown,
    /// Any other failure. The same file already passed the check without revocation, so this
    /// is unexpected, and it refuses.
    Failed(i32),
}

/// Sorts the HRESULT of a verification with revocation on (`None` when it passed).
pub(super) fn classify(error: Option<i32>) -> Status {
    match error {
        None => Status::Good,
        Some(REVOKED) => Status::Revoked,
        Some(NO_CHECK | OFFLINE | FAILED) => Status::Unknown,
        Some(code) => Status::Failed(code),
    }
}

/// Checks that `file`'s signing certificates aren't revoked. Call it only on a file whose
/// signature and publisher already passed.
///
/// shortcut: a check nobody can answer (offline, or no revocation server reachable) continues,
/// with a warning in the log, so all-users updates and downgrades still work offline. Upgrade to
/// a refusal if the publisher ever needs a withdrawn package refused on a machine that is offline.
#[cfg(windows)]
pub(super) fn check(file: &std::path::Path) -> Result<(), String> {
    let shown = file.display();
    match status(file, true)? {
        Status::Good => Ok(()),
        Status::Revoked => {
            Err(format!("{shown} was signed with a certificate its publisher has withdrawn, so it wasn't installed. Nothing was changed."))
        }
        Status::Unknown => {
            log::warn!("couldn't find out whether {shown}'s signing certificate was withdrawn (offline?), so it is installed without that check");
            Ok(())
        }
        Status::Failed(code) => Err(format!("couldn't check whether {shown}'s signature was withdrawn (error {code:#010x}), so it wasn't installed")),
    }
}

/// `file`'s signature checked with revocation on. `network` lets Windows fetch revocation lists;
/// without it, only what Windows has cached is used.
#[cfg(windows)]
fn status(file: &std::path::Path, network: bool) -> Result<Status, String> {
    use verifysign::{CodeSignVerifier, Revocation, VerifyPolicy};

    let shown = file.display();
    let verifier = CodeSignVerifier::new(VerifyPolicy::new().revocation(Revocation::Chain).allow_network(network))
        .map_err(|e| format!("couldn't check whether {shown}'s signature was withdrawn ({e})"))?;
    match verifier.verify_file(file) {
        Ok(_) => Ok(classify(None)),
        Err(verifysign::Error::OsError(code)) => Ok(classify(Some(code))),
        Err(e) => Err(format!("couldn't check whether {shown}'s signature was withdrawn ({e}), so it wasn't installed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_revoked_certificate_is_refused() {
        assert_eq!(classify(Some(0x8009_2010_u32.cast_signed())), Status::Revoked);
    }

    #[test]
    fn a_check_nobody_can_answer_is_unknown_not_revoked() {
        for code in [0x8009_2012_u32, 0x8009_2013, 0x800B_010E] {
            assert_eq!(classify(Some(code.cast_signed())), Status::Unknown, "{code:#x}");
        }
    }

    #[test]
    fn a_passed_check_is_good_and_any_other_error_refuses() {
        assert_eq!(classify(None), Status::Good);
        // TRUST_E_BAD_DIGEST: the file changed since it was signed.
        assert_eq!(classify(Some(0x8009_6010_u32.cast_signed())), Status::Failed(0x8009_6010_u32.cast_signed()));
    }

    /// A real signed package, when given (`ARTCRAFT_REVOCATION_REAL_MSI`): online, Windows must
    /// answer (not merely fail to); from its cache alone, it must never refuse.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs a real signed package and the network: set ARTCRAFT_REVOCATION_REAL_MSI"]
    fn a_real_package_is_not_revoked() {
        let file = std::path::PathBuf::from(std::env::var_os("ARTCRAFT_REVOCATION_REAL_MSI").unwrap_or_default());
        assert_eq!(status(&file, true), Ok(Status::Good));
        assert!(matches!(status(&file, false), Ok(Status::Good | Status::Unknown)), "{:?}", status(&file, false));
        assert_eq!(check(&file), Ok(()));
    }
}
