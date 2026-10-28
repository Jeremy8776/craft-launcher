//! The client end of the updater service's named pipe (Windows only).
//!
//! [`NoWait`] and [`Stream`] are public because the service reads its end of the pipe the same
//! way; there is no server code here.

use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use interprocess::os::windows::named_pipe::{DuplexPipeStream, pipe_mode};

use crate::wire::{Reply, frame, read_frame};

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

/// Sends `body` to the service and waits up to `timeout` for the answer. Fails at once when the
/// service is not running.
pub fn request(pipe_path: &str, body: &[u8], timeout: Duration) -> Result<Reply, String> {
    let mut stream = Stream::connect_by_path(Path::new(pipe_path)).map_err(|e| format!("couldn't reach the updater service ({e})"))?;
    stream.set_nonblocking(true).map_err(|e| format!("couldn't talk to the updater service ({e})"))?;
    if body.len() > CLIENT_MAX_BODY {
        return Err("the request was too large".into());
    }
    let framed = frame(body).ok_or("the request was too large")?;
    let mut rest = framed.as_slice();
    let until = Instant::now() + Duration::from_secs(5);
    while !rest.is_empty() {
        match stream.write(rest) {
            Ok(n) if n > 0 => rest = rest.get(n..).unwrap_or_default(),
            // `Ok(0)` and `WouldBlock` both mean "no room yet" on a non-blocking pipe.
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(format!("couldn't talk to the updater service ({e})")),
        }
        if !rest.is_empty() {
            if Instant::now() >= until {
                return Err("the updater service did not take the request".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let answer = read_frame(&mut NoWait(&mut stream), REPLY_MAX_BYTES, Instant::now() + timeout)
        .map_err(|e| format!("no answer from the updater service ({e:?})"))?;
    serde_json::from_slice(&answer).map_err(|_| "the updater service sent an answer that could not be read".to_string())
}
