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
//!
//! The pipe's name is fixed, so any local program could create a pipe with that name when the
//! service is not running and answer "installed". Two checks stop that from skipping an update:
//! the launcher only talks to the process Windows reports as the running updater service
//! ([`is_the_service`]), and it believes "installed" only after Windows Installer's own records
//! show the release's version ([`update`]).

use artcraft_launcher_core::release::Release;
use artcraft_launcher_core::version::Version;
use artcraft_launcher_core::{CraftApp, PackageKind};
use artcraft_launcher_updater_client::Answer;

use crate::install::MsiScope;

/// Where the answer comes from. The real one is the service's pipe; tests use a script.
pub(crate) trait Service {
    /// `cancelled` is asked while the answer is awaited; when it says so the wait ends at once.
    fn ask(&self, app_id: &str, version: &str, cancelled: &dyn Fn() -> bool) -> Answer;
}

/// The real updater service, over its named pipe.
#[cfg(windows)]
pub(crate) struct Pipe;

#[cfg(windows)]
impl Service for Pipe {
    fn ask(&self, app_id: &str, version: &str, cancelled: &dyn Fn() -> bool) -> Answer {
        artcraft_launcher_updater_client::ask_cancellable(app_id, version, &is_the_service, cancelled)
    }
}

/// Whether this install is one the service could update: an MSI update of an app that is already
/// installed for all users. A fresh install, a per-user copy, a reinstall of the same version and
/// every other package type go the normal way.
pub(crate) fn wanted(kind: PackageKind, scope: Option<MsiScope>, is_update: bool) -> bool {
    is_update && kind == PackageKind::Msi && scope == Some(MsiScope::PerMachine)
}

/// What came of asking the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Silent {
    /// The service said it finished AND Windows Installer's records show the release's version.
    Done,
    /// Nothing to rely on: carry on with the normal route.
    Normal,
    /// The person cancelled while the launcher waited. The service was not told to stop, so it may
    /// still finish the update: the normal route must not start a second install.
    Cancelled,
}

impl Silent {
    /// How the install job ends when the service has settled it: `Some(Ok)` when the update is
    /// done, `Some(Err)` (the job's own cancelled error) when the person cancelled, and `None` when
    /// the normal route has to run.
    pub(crate) fn settled(self) -> Option<Result<(), String>> {
        match self {
            Self::Done => Some(Ok(())),
            Self::Cancelled => Some(Err(artcraft_launcher_core::protocol::CANCELLED.to_string())),
            Self::Normal => None,
        }
    }
}

/// Asks the service to update `app` to `release`. [`Silent::Done`] only when the service said it
/// finished AND `installed_version` (Windows Installer's records, read again by the caller) now shows
/// the release's version. `cancelled` ends the wait early. Anything else is logged.
pub(crate) fn update(
    service: &dyn Service,
    installed_version: &dyn Fn() -> Option<Version>,
    app: &CraftApp,
    release: &Release,
    cancelled: &dyn Fn() -> bool,
) -> Silent {
    // The service is asked by version number. A release known only by its tag cannot be named.
    let Some(version) = release.version.as_ref() else {
        return Silent::Normal;
    };
    let answer = service.ask(app.id, &version.to_string(), cancelled);
    log::info!("{}: {}", app.name, answer.describe());
    if answer == Answer::WaitCancelled {
        return Silent::Cancelled;
    }
    if !answer.is_installed() {
        return Silent::Normal;
    }
    // An answer is only words. Trust the machine's records instead: if they do not show the new
    // version, the normal route still has to run.
    let now = installed_version();
    if now.as_ref() != Some(version) {
        log::warn!("{}: the updater service said installed, but Windows shows {:?}; updating the normal way", app.name, now);
        return Silent::Normal;
    }
    Silent::Done
}

/// The name the updater's installer gives the service. The launcher talks to nothing else.
#[cfg(windows)]
const SERVICE_NAME: &str = "ArtCraftUpdater";

/// `Ok` only if process `pid` is the running updater service, as Windows' service manager reports
/// it. A program that merely created a pipe with the same name has a different process id. (A
/// normal user cannot open the service's process to look at its program file, but may ask the
/// service manager which process is the service.) The `Err` text says why not, for the log.
#[cfg(windows)]
pub fn is_the_service(pid: u32) -> Result<(), String> {
    same_process(service_process_id()?, pid)
}

/// `Ok` when the pipe's owner `pid` is the service's own process `running`.
#[cfg(any(windows, test))]
fn same_process(running: u32, pid: u32) -> Result<(), String> {
    if running == pid { Ok(()) } else { Err(format!("process {pid} is not the updater service (that is process {running})")) }
}

/// The process id of the running updater service.
#[cfg(windows)]
fn service_process_id() -> Result<u32, String> {
    use windows_service::service::{ServiceAccess, ServiceState};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| format!("couldn't ask Windows about services ({e})"))?;
    let service =
        manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS).map_err(|e| format!("the updater service is not installed ({e})"))?;
    let status = service.query_status().map_err(|e| format!("couldn't read the updater service's state ({e})"))?;
    match (status.current_state, status.process_id) {
        (ServiceState::Running, Some(pid)) => Ok(pid),
        (state, _) => Err(format!("the updater service is not running ({state:?})")),
    }
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
        fn ask(&self, app_id: &str, version: &str, cancelled: &dyn Fn() -> bool) -> Answer {
            self.asked.borrow_mut().push((app_id.into(), version.into()));
            if cancelled() {
                return Answer::WaitCancelled;
            }
            self.answer.clone()
        }
    }

    fn cadcraft() -> &'static CraftApp {
        SUITE.iter().find(|app| app.id == "cadcraft").unwrap()
    }

    fn release(v: &str) -> Release {
        Release { version: version::parse(v), tag: format!("v{v}"), ..Release::default() }
    }

    /// Windows Installer's records after the update: the new version.
    fn shows(v: &'static str) -> impl Fn() -> Option<Version> {
        move || version::parse(v)
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
        assert_eq!(update(&service, &shows("1.2.3"), cadcraft(), &release("1.2.3"), &|| false), Silent::Done);
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
            // Even if the records happened to show the new version, only "installed" counts.
            assert_eq!(update(&service, &shows("1.2.3"), cadcraft(), &release("1.2.3"), &|| false), Silent::Normal, "{answer:?}");
            assert_eq!(service.asked.borrow().len(), 1, "{answer:?}");
        }
    }

    #[test]
    fn a_cancel_while_waiting_stops_the_job_and_does_not_start_a_second_install() {
        // The service would have said "installed", but the person cancelled first: the caller's own
        // cancel check has to reach the wait, and the result must not be the normal route.
        let service = Scripted::says(Answer::Installed);
        let result = update(&service, &shows("1.2.3"), cadcraft(), &release("1.2.3"), &|| true);
        assert_eq!(result, Silent::Cancelled);
        assert_eq!(service.asked.borrow().len(), 1);
    }

    #[test]
    fn only_a_normal_answer_lets_the_normal_route_run() {
        assert_eq!(Silent::Done.settled(), Some(Ok(())));
        assert_eq!(Silent::Cancelled.settled(), Some(Err(artcraft_launcher_core::protocol::CANCELLED.to_string())));
        assert_eq!(Silent::Normal.settled(), None);
    }

    #[test]
    fn a_release_without_a_version_number_is_not_sent() {
        let service = Scripted::says(Answer::Installed);
        let tag_only = Release { tag: "nightly".into(), ..Release::default() };
        assert_eq!(update(&service, &shows("1.2.3"), cadcraft(), &tag_only, &|| false), Silent::Normal);
        assert!(service.asked.borrow().is_empty());
    }

    #[test]
    fn installed_is_not_believed_unless_windows_shows_the_new_version() {
        // A program pretending to be the service says "installed" but nothing changed.
        let liar = Scripted::says(Answer::Installed);
        let lie = |installed: &dyn Fn() -> Option<Version>| update(&liar, installed, cadcraft(), &release("1.2.3"), &|| false);
        assert_eq!(lie(&shows("1.2.2")), Silent::Normal, "old version still installed");
        assert_eq!(lie(&|| None), Silent::Normal, "app no longer found");
        assert_eq!(lie(&shows("1.2.4")), Silent::Normal, "some other version");
    }

    #[test]
    fn only_the_services_own_process_may_own_the_pipe() {
        assert_eq!(same_process(4321, 4321), Ok(()));
        assert!(same_process(4321, 4322).is_err());
        assert!(same_process(4321, 0).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn this_test_program_is_not_the_updater_service() {
        // Whether or not a service is installed on this PC, this test program is not it.
        let this = is_the_service(std::process::id());
        assert!(this.is_err(), "{this:?}");
        let other = is_the_service(0xFFFF_FFF0);
        assert!(other.is_err(), "{other:?}");
    }
}
