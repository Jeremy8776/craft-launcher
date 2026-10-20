//! The launcher's local Select subject sample uses the installed SAM 2.1 session.
//! This keeps the selected model on its native path instead of passing image.segment
//! to ComfyUI's imported-workflow provider.
use std::time::Instant;

use craft_ai_comfyui::{Sam21Canceller, Sam21Session};

// Each sample owns a private copy of the loopback server helper so neither depends on the other.
#[allow(clippy::duplicate_mod)]
#[path = "features_sam3_server.rs"]
mod server;

pub(super) fn photo() -> &'static [u8] {
    include_bytes!("../../../assets/samples/sam3-astronaut.png")
}

pub(super) fn run(
    endpoint: &str,
    photo: &[u8],
    register: impl FnOnce(Sam21Canceller) -> Result<(), String>,
    still_active: impl Fn() -> bool,
) -> Result<(String, u64), String> {
    let started = Instant::now();
    server::ensure_running(endpoint)?;
    let mut session = Sam21Session::new(endpoint).map_err(|e| e.message)?;
    register(session.canceller())?;
    session.verify_server().map_err(|e| e.message)?;
    session.prepare(photo).map_err(|e| e.message)?;
    if !still_active() {
        return Err("SAM 2.1 request cancelled".to_owned());
    }
    let mask = session.ground("person").map_err(|e| e.message)?;
    if !mask.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("SAM 2.1 did not return a selection image".to_owned());
    }
    let millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(("Returned 1 image result(s)".to_owned(), millis))
}
