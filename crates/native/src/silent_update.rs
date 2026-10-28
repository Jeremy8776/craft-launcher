//! Asking the updater service to do a per-machine update, so Windows does not ask for permission
//! again for every update.
//!
//! The service (`artcraft-launcher-updater-service`) is optional. It exists only on a PC where the
//! person chose to turn on silent updates for all-user installs. This module's one promise: it can
//! make an update happen without a prompt, and it can never make an update fail. Every other
//! answer (no service, a decline, a refusal, an error) returns `false`, and the caller carries on
//! with the normal route, which downloads the installer and lets Windows ask for permission.
//!
//! Only an update of an app that is already installed for all users is ever sent. The service
//! decides everything else itself: it fetches the release, checks its checksum and signature,
//! and refuses anything that is not a newer official release of an app it can see.

use artcraft_launcher_core::release::Release;
use artcraft_launcher_core::{CraftApp, PackageKind};
use artcraft_launcher_updater_client::Answer;

use crate::install::MsiScope;

/// Where the answer comes from. The real one is the service's pipe; tests use a script.
pub(crate) trait Service {
    fn ask(&self, app_id: &str, version: &str) -> Answer;
}

/// The real updater service, over its named pipe.
#[cfg(windows)]
pub(crate) struct Pipe;

#[cfg(windows)]
impl Service for Pipe {
    fn ask(&self, app_id: &str, version: &str) -> Answer {
        artcraft_launcher_updater_client::ask(app_id, version)
    }
}

/// Whether this install is one the service could update: an MSI update of an app that is already
/// installed for all users. A fresh install, a per-user copy, a reinstall of the same version and
/// every other package type go the normal way.
pub(crate) fn wanted(kind: PackageKind, scope: Option<MsiScope>, is_update: bool) -> bool {
    is_update && kind == PackageKind::Msi && scope == Some(MsiScope::PerMachine)
}

/// Asks the service to update `app` to `release`. `true` only when the service finished the
/// update. Anything else is logged and returns `false`.
pub(crate) fn update(service: &dyn Service, app: &CraftApp, release: &Release) -> bool {
    // The service is asked by version number. A release known only by its tag cannot be named.
    let Some(version) = release.version.as_ref() else {
        return false;
    };
    let answer = service.ask(app.id, &version.to_string());
    log::info!("{}: {}", app.name, answer.describe());
    answer.is_installed()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use artcraft_launcher_core::SUITE;
    use artcraft_launcher_core::version;

    use super::*;

    struct Scripted {
        answer: Answer,
        asked: RefCell<Vec<(String, String)>>,
    }

    impl Scripted {
        fn says(answer: Answer) -> Self {
            Self { answer, asked: RefCell::new(Vec::new()) }
        }
    }

    impl Service for Scripted {
        fn ask(&self, app_id: &str, version: &str) -> Answer {
            self.asked.borrow_mut().push((app_id.into(), version.into()));
            self.answer.clone()
        }
    }

    fn cadcraft() -> &'static CraftApp {
        SUITE.iter().find(|app| app.id == "cadcraft").unwrap()
    }

    fn release(v: &str) -> Release {
        Release { version: version::parse(v), tag: format!("v{v}"), ..Release::default() }
    }

    #[test]
    fn only_an_update_of_an_all_users_msi_goes_to_the_service() {
        assert!(wanted(PackageKind::Msi, Some(MsiScope::PerMachine), true));
        // A fresh install, a per-user copy, an unknown scope and other packages stay on the normal route.
        assert!(!wanted(PackageKind::Msi, Some(MsiScope::PerMachine), false));
        assert!(!wanted(PackageKind::Msi, Some(MsiScope::PerUser), true));
        assert!(!wanted(PackageKind::Msi, None, true));
        assert!(!wanted(PackageKind::PortableZip, Some(MsiScope::PerMachine), true));
    }

    #[test]
    fn a_finished_update_is_reported_as_done_and_names_only_the_app_and_version() {
        let service = Scripted::says(Answer::Installed);
        assert!(update(&service, cadcraft(), &release("1.2.3")));
        assert_eq!(*service.asked.borrow(), vec![("cadcraft".to_string(), "1.2.3".to_string())]);
    }

    #[test]
    fn every_other_answer_leaves_the_update_to_the_normal_route() {
        for answer in [
            Answer::Busy,
            Answer::Cancelled,
            Answer::Failed(1603),
            Answer::Declined("installed outside Program Files".into()),
            Answer::Refused("that version is not newer than the one installed".into()),
            Answer::Unavailable("couldn't reach the updater service".into()),
        ] {
            let service = Scripted::says(answer.clone());
            assert!(!update(&service, cadcraft(), &release("1.2.3")), "{answer:?}");
            assert_eq!(service.asked.borrow().len(), 1, "{answer:?}");
        }
    }

    #[test]
    fn a_release_without_a_version_number_is_not_sent() {
        let service = Scripted::says(Answer::Installed);
        let tag_only = Release { tag: "nightly".into(), ..Release::default() };
        assert!(!update(&service, cadcraft(), &tag_only));
        assert!(service.asked.borrow().is_empty());
    }
}
