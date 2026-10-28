//! The pipe between the launcher (a normal user process) and the service (SYSTEM).
//!
//! Security, in order of how much each is worth:
//!
//! 1. The engine decides everything. A caller can only ask for "update app X to version Y", and
//!    the engine refuses unless that is an installed app, a newer official release, correctly
//!    checksummed and signed. So even a hostile caller can only cause a legitimate update.
//! 2. The pipe's security descriptor: SYSTEM and Administrators full control, interactive users
//!    read and write, nobody else, and no remote clients. Network logons and other service
//!    accounts can't open it.
//! 3. [`Callers`]: an optional check of the connecting process. `verify_pid` is documented as
//!    unreliable (PID reuse, and it checks the file on disk, not the running image), so
//!    [`SignedCaller`] is defence in depth only and the service does not need it for safety.
//!
//! One bad client can't hold the service: each connection is read on its own thread with a
//! deadline, at most [`MAX_READERS`] at once, and the install itself runs on the calling thread.

use std::io::{self, Read, Write};
use std::num::NonZeroU8;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use interprocess::os::windows::named_pipe::{DuplexPipeStream, PipeListenerOptions, PipeMode, pipe_mode};
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use widestring::U16CString;

use crate::error::{Outcome, Refusal};
use crate::publisher::PUBLISHER_ORGANISATIONS;
use crate::request::MAX_REQUEST_BYTES;
use crate::wire::{FrameError, Reply, frame, read_frame, reply_frame};

type Stream = DuplexPipeStream<pipe_mode::Bytes>;

/// Reads a non-blocking pipe. Windows answers a non-blocking read with no data yet as `Ok(0)`,
/// which would look like a closed connection. A real disconnect arrives as a `BrokenPipe` error,
/// so `Ok(0)` is turned into `WouldBlock` here and the callers' deadlines end the wait.
struct NoWait<'a>(&'a mut Stream);

impl Read for NoWait<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buf) {
            Ok(0) if !buf.is_empty() => Err(io::ErrorKind::WouldBlock.into()),
            other => other,
        }
    }
}

/// The pipe's name.
pub const PIPE_PATH: &str = r"\\.\pipe\artcraft-updater";

/// SYSTEM and Administrators: full control. Interactive users: read and write. Protected, so
/// nothing is inherited. Nobody else is listed, so everyone else is refused.
const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";

/// Pipe buffer size, and the largest request the client will try to send. The service refuses
/// anything over [`MAX_REQUEST_BYTES`] itself; this only has to be bigger.
const BUFFER_BYTES: u32 = 8192;
const CLIENT_MAX_BODY: usize = 4096;

/// Connections being read at once. More than this are dropped at once.
pub const MAX_READERS: usize = 16;

/// How the server behaves. [`Config::default`] is what the service uses.
pub struct Config {
    pub pipe_path: String,
    /// How long a client has to send its whole request.
    pub read_deadline: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self { pipe_path: PIPE_PATH.to_string(), read_deadline: Duration::from_secs(5) }
    }
}

/// Decides whether the process on the other end of the pipe may talk to the service at all.
pub trait Callers: Sync {
    fn allow(&self, pid: u32) -> bool;
}

/// Lets every local process that can open the pipe through. Safe, because of the engine.
pub struct AnyCaller;
impl Callers for AnyCaller {
    fn allow(&self, _: u32) -> bool {
        true
    }
}

/// Requires the caller's exe to be validly signed by our publisher. Defence in depth only.
pub struct SignedCaller;
impl Callers for SignedCaller {
    fn allow(&self, pid: u32) -> bool {
        verifysign::verify_pid(pid)
            .ok()
            .and_then(|signature| signature.subject_name().organization)
            .is_some_and(|org| PUBLISHER_ORGANISATIONS.contains(&org.as_str()))
    }
}

/// How the pipe is created: restricted to the descriptor above, local only. A function of its own
/// so a test can read the settings back.
fn listener_options(pipe_path: &str) -> io::Result<PipeListenerOptions<'_>> {
    let sddl = U16CString::from_str(PIPE_SDDL).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    Ok(PipeListenerOptions::new()
        .path(Path::new(pipe_path))
        .mode(PipeMode::Bytes)
        .nonblocking(true)
        .accept_remote(false)
        // Room for a whole request (capped at 1 KiB) and a reply: a non-blocking write that doesn't
        // fit is not partly written, so the default buffer would leave a larger request stuck.
        .input_buffer_size_hint(BUFFER_BYTES)
        .output_buffer_size_hint(BUFFER_BYTES)
        .instance_limit(NonZeroU8::new(32))
        .security_descriptor(Some(SecurityDescriptor::deserialize(&sddl)?)))
}

/// Serves requests until `stop` is set. `handler` runs on this thread, one request at a time
/// (Windows Installer allows only one install at once anyway).
pub fn serve(config: &Config, handler: &dyn Fn(&[u8]) -> Result<Outcome, Refusal>, callers: &dyn Callers, stop: &AtomicBool) -> io::Result<()> {
    let listener = listener_options(&config.pipe_path)?.create_duplex::<pipe_mode::Bytes>()?;

    let live = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel::<(Stream, Vec<u8>)>();
    std::thread::scope(|scope| {
        let (live, tx) = (&live, tx);
        scope.spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok(stream) if live.fetch_add(1, Ordering::SeqCst) < MAX_READERS => {
                        let tx = tx.clone();
                        scope.spawn(move || {
                            read_one(stream, config.read_deadline, callers, &tx);
                            live.fetch_sub(1, Ordering::SeqCst);
                        });
                    }
                    Ok(_dropped) => {
                        live.fetch_sub(1, Ordering::SeqCst);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(20)),
                    Err(_) => std::thread::sleep(Duration::from_millis(200)),
                }
            }
        });
        while !stop.load(Ordering::SeqCst) {
            if let Ok((mut stream, body)) = rx.recv_timeout(Duration::from_millis(100)) {
                let reply = Reply::from(&handler(&body));
                // Delivered on its own thread, so a client that never reads can't hold up the next install.
                scope.spawn(move || send(&mut stream, &reply));
            }
        }
    });
    Ok(())
}

/// Checks the caller, reads one request, and queues it for the handler. Anything wrong is
/// answered here and nothing is queued.
fn read_one(mut stream: Stream, deadline: Duration, callers: &dyn Callers, queue: &mpsc::Sender<(Stream, Vec<u8>)>) {
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    if !stream.client_process_id().is_ok_and(|pid| callers.allow(pid)) {
        return send(&mut stream, &Reply::Refused { reason: "this program may not use the updater service".into() });
    }
    match read_frame(&mut NoWait(&mut stream), MAX_REQUEST_BYTES, Instant::now() + deadline) {
        Ok(body) => {
            let _ = queue.send((stream, body));
        }
        Err(FrameError::TooLarge) => send(&mut stream, &Reply::from(&Err(Refusal::TooLarge))),
        Err(FrameError::TimedOut) => send(&mut stream, &Reply::Refused { reason: "the request took too long to arrive".into() }),
        Err(_) => {}
    }
}

/// Writes a reply. The client is local and the reply is tiny; retry briefly if the pipe is busy.
fn send(stream: &mut Stream, reply: &Reply) {
    let bytes = reply_frame(reply);
    let until = Instant::now() + Duration::from_secs(2);
    let mut rest = bytes.as_slice();
    while !rest.is_empty() && Instant::now() < until {
        match stream.write(rest) {
            // A non-blocking write that doesn't fit yet is `Ok(0)`, like a read with no data.
            Ok(0) => std::thread::sleep(Duration::from_millis(5)),
            Ok(n) => rest = rest.get(n..).unwrap_or_default(),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break,
        }
    }
    let _ = stream.flush();
    // Closing a pipe discards what the client has not read yet. Wait for the client to read its
    // answer and close, which our client does at once, but never longer than a couple of seconds.
    let mut probe = [0u8; 1];
    while Instant::now() < until {
        match NoWait(stream).read(&mut probe) {
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break,
        }
    }
}

/// Sends `body` to the service and waits up to `timeout` for the answer. This is what the
/// launcher calls.
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
    let answer =
        read_frame(&mut NoWait(&mut stream), 4096, Instant::now() + timeout).map_err(|e| format!("no answer from the updater service ({e:?})"))?;
    serde_json::from_slice(&answer).map_err(|_| "the updater service sent an answer that could not be read".to_string())
}

#[cfg(test)]
mod tests;
