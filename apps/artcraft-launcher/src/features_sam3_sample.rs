//! The launcher's local Select subject sample uses PhotoCraft's SAM 3 session path.
//! Imported ComfyUI workflows cannot carry an image.segment request, so a file library
//! must never be taken as proof that this selected model can segment a picture.
use std::time::Instant;

use craft_ai_comfyui::{Sam3Canceller, Sam3Session};

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
    register: impl FnOnce(Sam3Canceller) -> Result<(), String>,
    still_active: impl Fn() -> bool,
) -> Result<(String, u64), String> {
    let started = Instant::now();
    server::ensure_running(endpoint)?;
    let mut session = Sam3Session::new(endpoint).map_err(|e| e.message)?;
    register(session.canceller())?;
    session.verify_server().map_err(|e| e.message)?;
    session.prepare(photo).map_err(|e| e.message)?;
    if !still_active() {
        return Err("SAM 3 request cancelled".to_owned());
    }
    // `ground` rejects an empty mask and checks that the result fits the prepared image.
    let mask = session.ground("person").map_err(|e| e.message)?;
    if !mask.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("SAM 3 did not return a selection image".to_owned());
    }
    let millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(("Returned 1 image result(s)".to_owned(), millis))
}
