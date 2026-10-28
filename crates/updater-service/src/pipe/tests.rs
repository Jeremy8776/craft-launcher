//! Pipe tests: a real server on a real named pipe, in this process, with a scripted handler.
//! (The ACL's refusal of other accounts needs a second Windows account and is not covered here.)

use std::sync::atomic::AtomicU32;

use super::*;

static NEXT: AtomicU32 = AtomicU32::new(0);

fn config(deadline_ms: u64) -> Config {
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    Config { pipe_path: format!(r"\\.\pipe\artcraft-updater-test-{}-{n}", std::process::id()), read_deadline: Duration::from_millis(deadline_ms) }
}

struct Never;
impl Callers for Never {
    fn allow(&self, _: u32) -> bool {
        false
    }
}

/// Stops the server when dropped, so a test that panics can't leave `serve` running forever.
struct StopOnDrop<'a>(&'a AtomicBool);
impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Runs `serve` on a thread while `with` talks to it, then stops it.
fn with_server<R>(
    config: &Config,
    handler: &(dyn Fn(&[u8]) -> Result<Outcome, Refusal> + Sync),
    callers: &dyn Callers,
    with: impl FnOnce(&str) -> R,
) -> R {
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        let _guard = StopOnDrop(&stop);
        let server = s.spawn(|| serve(config, handler, callers, &stop));
        // Wait for the pipe to exist (the server creates it before it accepts).
        let up = Instant::now() + Duration::from_secs(5);
        while Stream::connect_by_path(Path::new(&config.pipe_path)).is_err() && Instant::now() < up {
            std::thread::sleep(Duration::from_millis(10));
        }
        let result = with(&config.pipe_path);
        stop.store(true, Ordering::SeqCst);
        server.join().unwrap().unwrap();
        result
    })
}

#[test]
fn a_request_goes_through_and_the_answer_comes_back() {
    let handler = |body: &[u8]| {
        assert_eq!(body, br#"{"app":"cadcraft","version":"1.2.3"}"#);
        Ok(Outcome::Installed)
    };
    let reply = with_server(&config(2000), &handler, &AnyCaller, |path| {
        request(path, br#"{"app":"cadcraft","version":"1.2.3"}"#, Duration::from_secs(5)).unwrap()
    });
    assert_eq!(reply, Reply::Ran { outcome: Outcome::Installed });
}

#[test]
fn a_refusal_comes_back_as_plain_text() {
    let handler = |_: &[u8]| Err(Refusal::NotNewer);
    let reply = with_server(&config(2000), &handler, &AnyCaller, |path| request(path, b"{}", Duration::from_secs(5)).unwrap());
    assert!(matches!(reply, Reply::Refused { reason } if reason.contains("not newer")));
}

#[test]
fn a_caller_that_is_not_allowed_never_reaches_the_handler() {
    let calls = AtomicUsize::new(0);
    let handler = |_: &[u8]| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(Outcome::Installed)
    };
    let reply = with_server(&config(2000), &handler, &Never, |path| request(path, b"{}", Duration::from_secs(5)).unwrap());
    assert!(matches!(reply, Reply::Refused { .. }));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn this_unsigned_test_program_is_not_a_signed_caller() {
    // The real check, on a real process: the test exe is not signed by the publisher.
    assert!(!SignedCaller.allow(std::process::id()));
}

#[test]
fn an_oversize_request_is_refused_without_reaching_the_handler() {
    let calls = AtomicUsize::new(0);
    let handler = |_: &[u8]| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(Outcome::Installed)
    };
    let big = vec![b' '; MAX_REQUEST_BYTES + 1];
    let reply = with_server(&config(2000), &handler, &AnyCaller, |path| {
        // Over the service's limit but small enough for the client to send, then far too big for the client.
        assert!(request(path, &vec![b' '; 5000], Duration::from_secs(5)).is_err());
        request(path, &big, Duration::from_secs(5)).unwrap()
    });
    assert!(matches!(reply, Reply::Refused { reason } if reason.contains("too large")));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_silent_client_does_not_block_a_good_one() {
    let handler = |_: &[u8]| Ok(Outcome::Installed);
    let reply = with_server(&config(1500), &handler, &AnyCaller, |path| {
        // Connect and say nothing. Keep the connection open while the good client is served.
        let _silent = Stream::connect_by_path(Path::new(path)).unwrap();
        let started = Instant::now();
        let reply = request(path, b"{}", Duration::from_secs(5)).unwrap();
        assert!(started.elapsed() < Duration::from_millis(1200), "the good client waited for the silent one");
        reply
    });
    assert_eq!(reply, Reply::Ran { outcome: Outcome::Installed });
}

#[test]
fn a_half_frame_times_out_and_the_service_keeps_serving() {
    let handler = |_: &[u8]| Ok(Outcome::Installed);
    let reply = with_server(&config(200), &handler, &AnyCaller, |path| {
        let mut half = Stream::connect_by_path(Path::new(path)).unwrap();
        half.write_all(&[0, 40, b'{']).unwrap(); // promises 40 bytes, sends one
        std::thread::sleep(Duration::from_millis(500));
        request(path, b"{}", Duration::from_secs(5)).unwrap()
    });
    assert_eq!(reply, Reply::Ran { outcome: Outcome::Installed });
}

#[test]
fn requests_are_handled_one_at_a_time() {
    let running = AtomicUsize::new(0);
    let overlap = AtomicBool::new(false);
    let handler = |_: &[u8]| {
        if running.fetch_add(1, Ordering::SeqCst) > 0 {
            overlap.store(true, Ordering::SeqCst);
        }
        std::thread::sleep(Duration::from_millis(80));
        running.fetch_sub(1, Ordering::SeqCst);
        Ok(Outcome::Installed)
    };
    with_server(&config(2000), &handler, &AnyCaller, |path| {
        std::thread::scope(|s| {
            let calls: Vec<_> = (0..4).map(|_| s.spawn(|| request(path, b"{}", Duration::from_secs(10)).unwrap())).collect();
            for c in calls {
                assert_eq!(c.join().unwrap(), Reply::Ran { outcome: Outcome::Installed });
            }
        });
    });
    assert!(!overlap.load(Ordering::SeqCst), "two installs ran at once");
}

#[test]
fn the_pipe_is_local_only_and_restricted() {
    // Not a network test: read the creation settings back. Pin the descriptor text so a change
    // to who may open the pipe has to be a decision.
    let options = listener_options(PIPE_PATH).unwrap();
    assert!(!options.accept_remote, "remote machines could open the pipe");
    assert!(options.security_descriptor.is_some(), "the pipe would take the default descriptor");
    assert_eq!(PIPE_SDDL, "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)");
}
