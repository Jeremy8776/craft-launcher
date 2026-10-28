//! The question the launcher asks, and the answer reduced to what the launcher does next.

#[cfg(windows)]
use std::time::Duration;

use serde::Serialize;

use crate::outcome::Outcome;
use crate::wire::Reply;

/// How long the launcher waits for the service to finish an update. The service downloads the
/// installer itself, so this covers a slow connection and a large app.
#[cfg(windows)]
const ANSWER_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The only two fields a request has. The service refuses any other.
#[derive(Serialize)]
struct Request<'a> {
    app: &'a str,
    version: &'a str,
}

/// The request for "update `app_id` to `version`".
pub fn request_body(app_id: &str, version: &str) -> Vec<u8> {
    // Two short strings always serialise; the empty body would be refused as malformed.
    serde_json::to_vec(&Request { app: app_id, version }).unwrap_or_default()
}

/// What came of asking the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The update was installed. Nothing more to do.
    Installed,
    /// Another install was running, so nothing was done.
    Busy,
    /// The install was cancelled or interrupted.
    Cancelled,
    /// Windows Installer stopped with this error code.
    Failed(i32),
    /// The service leaves this app to the normal update route (for example it sits outside
    /// Program Files). Not an error.
    Declined(String),
    /// The service refused the request. The reason is plain text.
    Refused(String),
    /// The service is not installed, not running, or did not answer properly.
    Unavailable(String),
}

impl Answer {
    /// Whether the update is done. For every other answer the launcher uses its normal route.
    pub fn is_installed(&self) -> bool {
        matches!(self, Self::Installed)
    }

    /// A line for the log. Not for the person using the launcher.
    pub fn describe(&self) -> String {
        match self {
            Self::Installed => "installed by the updater service".into(),
            Self::Busy => "the updater service found another install running".into(),
            Self::Cancelled => "the updater service's install was cancelled".into(),
            Self::Failed(code) => format!("the updater service's install stopped with error {code}"),
            Self::Declined(why) => format!("the updater service left this to the normal update ({why})"),
            Self::Refused(why) => format!("the updater service refused ({why})"),
            Self::Unavailable(why) => format!("the updater service is not available ({why})"),
        }
    }
}

/// Turns what the pipe returned into an [`Answer`].
pub fn interpret(result: Result<Reply, String>) -> Answer {
    match result {
        Ok(Reply::Ran { outcome: Outcome::Installed | Outcome::InstalledRestartNeeded }) => Answer::Installed,
        Ok(Reply::Ran { outcome: Outcome::Busy }) => Answer::Busy,
        Ok(Reply::Ran { outcome: Outcome::Cancelled }) => Answer::Cancelled,
        Ok(Reply::Ran { outcome: Outcome::Failed(code) }) => Answer::Failed(code),
        Ok(Reply::Declined { reason }) => Answer::Declined(reason),
        Ok(Reply::Refused { reason }) => Answer::Refused(reason),
        Err(why) => Answer::Unavailable(why),
    }
}

/// Asks the updater service to update `app_id` to `version`, and waits for it to finish.
/// `server_ok` is given the process id of the program that owns the pipe and must say `Ok` only if it
/// is the real service (its `Err` text says why not); nothing is sent to anything else.
#[cfg(windows)]
pub fn ask(app_id: &str, version: &str, server_ok: &dyn Fn(u32) -> Result<(), String>) -> Answer {
    interpret(crate::pipe::request_verified(crate::pipe::PIPE_PATH, &request_body(app_id, version), ANSWER_TIMEOUT, server_ok))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_has_an_app_and_a_version_and_nothing_else() {
        assert_eq!(String::from_utf8(request_body("cadcraft", "1.2.3")).unwrap(), r#"{"app":"cadcraft","version":"1.2.3"}"#);
    }

    #[test]
    fn only_a_finished_install_counts_as_done() {
        let ran = |outcome| Ok(Reply::Ran { outcome });
        assert_eq!(interpret(ran(Outcome::Installed)), Answer::Installed);
        // Windows wants a restart, but the new version is installed.
        assert_eq!(interpret(ran(Outcome::InstalledRestartNeeded)), Answer::Installed);
        assert_eq!(interpret(ran(Outcome::Busy)), Answer::Busy);
        assert_eq!(interpret(ran(Outcome::Cancelled)), Answer::Cancelled);
        assert_eq!(interpret(ran(Outcome::Failed(1603))), Answer::Failed(1603));
        for not_done in [
            interpret(ran(Outcome::Busy)),
            interpret(ran(Outcome::Cancelled)),
            interpret(ran(Outcome::Failed(1603))),
            interpret(Ok(Reply::Declined { reason: "outside Program Files".into() })),
            interpret(Ok(Reply::Refused { reason: "not newer".into() })),
            interpret(Err("couldn't reach the updater service".into())),
        ] {
            assert!(!not_done.is_installed(), "{not_done:?}");
        }
        assert!(Answer::Installed.is_installed());
    }

    #[test]
    fn a_decline_a_refusal_and_a_missing_service_stay_apart() {
        assert_eq!(interpret(Ok(Reply::Declined { reason: "d".into() })), Answer::Declined("d".into()));
        assert_eq!(interpret(Ok(Reply::Refused { reason: "r".into() })), Answer::Refused("r".into()));
        assert_eq!(interpret(Err("e".into())), Answer::Unavailable("e".into()));
    }

    #[test]
    fn every_answer_has_a_log_line() {
        for answer in [
            Answer::Installed,
            Answer::Busy,
            Answer::Cancelled,
            Answer::Failed(1603),
            Answer::Declined("d".into()),
            Answer::Refused("r".into()),
            Answer::Unavailable("u".into()),
        ] {
            assert!(!answer.describe().is_empty());
        }
        assert!(Answer::Failed(1603).describe().contains("1603"));
    }
}
