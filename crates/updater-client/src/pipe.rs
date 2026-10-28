//! The client end of the updater service's named pipe (Windows only).
//!
//! [`NoWait`] and [`Stream`] are public because the service reads its end of the pipe the same
//! way; there is no server code here.

use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use interprocess::os::windows::named_pipe::{DuplexPipeStream, pipe_mode};

use crate::answer::WaitError;
use crate::wire::{FrameError, Reply, frame, read_frame_cancellable};

/// One end of the pipe.
pub type Stream = DuplexPipeStream<pipe_mode::Bytes>;

/// The pipe's name.
pub const PIPE_PATH: &str = r"\\.\pipe\artcraft-updater";

/// The largest request the client will try to send. The service refuses anything over its own,
/// smaller limit; this only has to be bigger.
const CLIENT_MAX_BODY: usize = 4096;

/// The largest reply the client will read.
const REPLY_MAX_BYTES: usize = 4096;

/// Reads a non-blocking pipe. Windows answers a non-blocking read with no data yet as `Ok(0)`,
/// which would look like a closed connection. A real disconnect arrives as a `BrokenPipe` error,
/// so `Ok(0)` is turned into `WouldBlock` here and the callers' deadlines end the wait.
pub struct NoWait<'a>(pub &'a mut Stream);

impl Read for NoWait<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buf) {
            Ok(0) if !buf.is_empty() => Err(io::ErrorKind::WouldBlock.into()),
            other => other,
        }
    }
}

/// [`request_verified`] without a check of who is on the other end. For tests, which run their own
/// server in this process. The launcher must use [`request_verified`] (through `ask`).
pub fn request(pipe_path: &str, body: &[u8], timeout: Duration) -> Result<Reply, String> {
    request_verified(pipe_path, body, timeout, &|_| Ok(()))
}

/// Sends `body` to the service and waits up to `timeout` for the answer. Fails at once when the
/// service is not running.
///
/// The pipe's name is fixed, so any local program can create a pipe with it when the service is not
/// running. `server_ok` is given the process id of the program that owns the pipe, and nothing is
/// sent unless it says `Ok`; its `Err` text says why not.
pub fn request_verified(pipe_path: &str, body: &[u8], timeout: Duration, server_ok: &dyn Fn(u32) -> Result<(), String>) -> Result<Reply, String> {
    request_cancellable(pipe_path, body, timeout, server_ok, &|| false).map_err(|e| e.to_string())
}

fn other(text: impl Into<String>) -> WaitError {
    WaitError::Other(text.into())
}

/// [`request_verified`] that also stops waiting for the answer, with [`WaitError::Cancelled`], as
/// soon as `cancelled` says so. Cancelling ends only this wait: a request the service already took
/// is not withdrawn, so the install it started goes on.
pub fn request_cancellable(
    pipe_path: &str,
    body: &[u8],
    timeout: Duration,
    server_ok: &dyn Fn(u32) -> Result<(), String>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Reply, WaitError> {
    let mut stream = Stream::connect_by_path(Path::new(pipe_path)).map_err(|e| other(format!("couldn't reach the updater service ({e})")))?;
    let server = stream.server_process_id().map_err(|e| other(format!("couldn't tell what is on the other end of the updater pipe ({e})")))?;
    if let Err(why) = server_ok(server) {
        return Err(other(format!("the program on the updater pipe is not the updater service ({why})")));
    }
    stream.set_nonblocking(true).map_err(|e| other(format!("couldn't talk to the updater service ({e})")))?;
    if body.len() > CLIENT_MAX_BODY {
        return Err(other("the request was too large"));
    }
    let framed = frame(body).ok_or_else(|| other("the request was too large"))?;
    let mut rest = framed.as_slice();
    let until = Instant::now() + Duration::from_secs(5);
    while !rest.is_empty() {
        match stream.write(rest) {
            Ok(n) if n > 0 => rest = rest.get(n..).unwrap_or_default(),
            // `Ok(0)` and `WouldBlock` both mean "no room yet" on a non-blocking pipe.
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(other(format!("couldn't talk to the updater service ({e})"))),
        }
        if !rest.is_empty() {
            if Instant::now() >= until {
                return Err(other("the updater service did not take the request"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let answer = read_frame_cancellable(&mut NoWait(&mut stream), REPLY_MAX_BYTES, Instant::now() + timeout, cancelled).map_err(|e| match e {
        FrameError::Cancelled => WaitError::Cancelled,
        failed => other(format!("no answer from the updater service ({failed:?})")),
    })?;
    serde_json::from_slice(&answer).map_err(|_| other("the updater service sent an answer that could not be read"))
}
