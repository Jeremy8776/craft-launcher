//! What goes over the pipe. The framing and the reply type are shared with the launcher and live
//! in `artcraft-launcher-updater-client`; the one piece that stays here is turning the engine's
//! result into a reply, because [`Refusal`] belongs to the service.

pub use artcraft_launcher_updater_client::wire::{FrameError, Reply, frame, read_frame, reply_frame};

use crate::error::{Outcome, Refusal};

/// The reply for what the engine decided. A decline is told apart from a refusal, so the launcher
/// can tell "use your normal route" from "this was not allowed".
pub fn reply_for(result: &Result<Outcome, Refusal>) -> Reply {
    match result {
        Ok(outcome) => Reply::Ran { outcome: *outcome },
        Err(Refusal::Declined(why)) => Reply::Declined { reason: why.clone() },
        Err(refusal) => Reply::Refused { reason: refusal.to_string() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engines_result_becomes_the_right_reply() {
        let ok = reply_for(&Ok(Outcome::Failed(1603)));
        assert_eq!(String::from_utf8(serde_json::to_vec(&ok).unwrap()).unwrap(), r#"{"result":"ran","outcome":{"failed":1603}}"#);
        let no = reply_for(&Err(Refusal::NotNewer));
        assert!(matches!(&no, Reply::Refused { reason } if reason.contains("not newer")));
        let polite = reply_for(&Err(Refusal::Declined("installed outside Program Files".into())));
        assert_eq!(serde_json::to_string(&polite).unwrap(), r#"{"result":"declined","reason":"installed outside Program Files"}"#);
    }

    /// The launcher's request is accepted by the service's own parser: a drift between the two
    /// crates would otherwise show up only on a real PC.
    #[test]
    fn the_launchers_request_is_one_the_service_accepts() {
        let body = artcraft_launcher_updater_client::request_body("cadcraft", "1.2.3");
        let request = crate::request::parse(&body).unwrap();
        assert_eq!((request.app.id, request.version.to_string()), ("cadcraft", "1.2.3".to_string()));
    }
}
