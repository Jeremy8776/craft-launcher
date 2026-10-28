//! The one message a caller can send: which app, which version.

use artcraft_launcher_core::catalog::AppKind;
use artcraft_launcher_core::{CraftApp, SUITE};
use serde::Deserialize;

use crate::error::Refusal;

/// A request is a couple of short strings. Anything bigger is not from our launcher.
pub const MAX_REQUEST_BYTES: usize = 1024;

/// A version longer than this is not a Craft App version.
const MAX_VERSION_LEN: usize = 64;

/// A request that passed every check. It can only name a catalog app and a plain version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRequest {
    pub app: &'static CraftApp,
    pub version: semver::Version,
}

/// The wire form. `deny_unknown_fields` means a caller can't smuggle in a path or an argument:
/// there is no field to put one in, and an extra field is refused, not ignored.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    app: String,
    version: String,
}

/// Reads and checks a request.
pub fn parse(bytes: &[u8]) -> Result<InstallRequest, Refusal> {
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(Refusal::TooLarge);
    }
    let wire: Wire = serde_json::from_slice(bytes).map_err(|_| Refusal::Malformed)?;
    // Only the apps in the catalog. The launcher updates itself another way, so its id is not
    // in `SUITE` and is refused here.
    let app = SUITE.iter().find(|a| a.id == wire.app).ok_or(Refusal::UnknownApp)?;
    // A community app lives in someone else's repository and is signed by someone else. SYSTEM
    // fetches from `storytold` only, so it is left to the launcher's own route.
    if app.kind != AppKind::Official {
        return Err(Refusal::Declined("this is a community app, updated the normal way".into()));
    }
    if wire.version.len() > MAX_VERSION_LEN {
        return Err(Refusal::BadVersion);
    }
    let version = semver::Version::parse(&wire.version).map_err(|_| Refusal::BadVersion)?;
    Ok(InstallRequest { app, version })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(app: &str, version: &str) -> Vec<u8> {
        // Built with serde_json so hostile characters reach the check under test instead of
        // breaking the JSON first.
        serde_json::to_vec(&serde_json::json!({ "app": app, "version": version })).unwrap()
    }

    #[test]
    fn accepts_every_official_app() {
        for app in SUITE.iter().filter(|a| a.kind == AppKind::Official) {
            let parsed = parse(&req(app.id, "1.2.3")).unwrap();
            assert_eq!(parsed.app.id, app.id);
            assert_eq!(parsed.version.to_string(), "1.2.3");
        }
    }

    #[test]
    fn declines_a_community_app_instead_of_fetching_it() {
        let community: Vec<_> = SUITE.iter().filter(|a| a.kind == AppKind::Community).collect();
        assert!(!community.is_empty(), "the catalog has no community app to test with");
        for app in community {
            let result = parse(&req(app.id, "1.2.3"));
            assert!(matches!(&result, Err(r) if r.is_decline()), "{}: {result:?}", app.id);
        }
    }

    #[test]
    fn refuses_the_launcher_and_strangers() {
        assert_eq!(parse(&req("artcraft-launcher", "1.0.0")), Err(Refusal::UnknownApp));
        assert_eq!(parse(&req("notepad", "1.0.0")), Err(Refusal::UnknownApp));
        assert_eq!(parse(&req("CADCRAFT", "1.0.0")), Err(Refusal::UnknownApp));
        assert_eq!(parse(&req("..\\cadcraft", "1.0.0")), Err(Refusal::UnknownApp));
    }

    #[test]
    fn refuses_a_field_that_could_carry_a_path_or_argument() {
        for extra in [r#""path":"C:\\evil.msi""#, r#""args":"/x""#, r#""url":"https://evil.example/x.msi""#] {
            let body = format!(r#"{{"app":"cadcraft","version":"1.0.0",{extra}}}"#);
            assert_eq!(parse(body.as_bytes()), Err(Refusal::Malformed), "{extra}");
        }
    }

    #[test]
    fn refuses_bad_versions() {
        for v in ["", "1", "1.0", "latest", "1.0.0 /qn", "1.0.0\"", "../1.0.0", &"9".repeat(100)] {
            assert!(matches!(parse(&req("cadcraft", v)), Err(Refusal::BadVersion)), "{v}");
        }
    }

    #[test]
    fn refuses_oversize_and_garbage() {
        assert_eq!(parse(&vec![b' '; MAX_REQUEST_BYTES + 1]), Err(Refusal::TooLarge));
        assert_eq!(parse(b"not json"), Err(Refusal::Malformed));
        assert_eq!(parse(b"{}"), Err(Refusal::Malformed));
        assert_eq!(parse(b""), Err(Refusal::Malformed));
    }
}
