//! What goes over the pipe: one framed request, one framed reply.
//!
//! A frame is a 2-byte big-endian length, then that many bytes. The reader refuses a frame longer
//! than it was told to expect before it reads the body, and gives up at a deadline, so a client
//! that connects and says nothing (or says half a frame) can't hold the service.

use std::io::{self, Read};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{Outcome, Refusal};

/// Why a frame could not be read.
#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The announced length is over the limit. Nothing more was read.
    TooLarge,
    /// The client stopped sending before the frame was complete.
    Closed,
    /// The deadline passed.
    TimedOut,
    /// The pipe failed.
    Io(io::ErrorKind),
}

/// `payload` with its length in front. `None` when it does not fit in a frame.
pub fn frame(payload: &[u8]) -> Option<Vec<u8>> {
    let len = u16::try_from(payload.len()).ok()?;
    let mut out = Vec::with_capacity(payload.len() + 2);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

/// Reads one frame of at most `max` bytes. `input` may be non-blocking: a `WouldBlock` is
/// retried until `deadline`.
pub fn read_frame(input: &mut dyn Read, max: usize, deadline: Instant) -> Result<Vec<u8>, FrameError> {
    let mut head = [0u8; 2];
    fill(input, &mut head, deadline)?;
    let len = usize::from(u16::from_be_bytes(head));
    if len > max {
        return Err(FrameError::TooLarge);
    }
    let mut body = vec![0u8; len];
    fill(input, &mut body, deadline)?;
    Ok(body)
}

fn fill(input: &mut dyn Read, mut buf: &mut [u8], deadline: Instant) -> Result<(), FrameError> {
    while !buf.is_empty() {
        match input.read(buf) {
            Ok(0) => return Err(FrameError::Closed),
            Ok(n) => buf = buf.get_mut(n..).ok_or(FrameError::Io(io::ErrorKind::InvalidData))?,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted => {
                if Instant::now() >= deadline {
                    return Err(FrameError::TimedOut);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(FrameError::Io(e.kind())),
        }
    }
    Ok(())
}

/// The service's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Reply {
    /// The installer ran. `outcome` says how it ended.
    Ran { outcome: Outcome },
    /// Nothing was installed. `reason` is plain text, safe to show.
    Refused { reason: String },
    /// Nothing was installed because the service leaves this one to the launcher's normal update
    /// route. Not an error: the launcher carries on with that route.
    Declined { reason: String },
}

impl From<&Result<Outcome, Refusal>> for Reply {
    fn from(result: &Result<Outcome, Refusal>) -> Self {
        match result {
            Ok(outcome) => Self::Ran { outcome: *outcome },
            Err(Refusal::Declined(why)) => Self::Declined { reason: why.clone() },
            Err(refusal) => Self::Refused { reason: refusal.to_string() },
        }
    }
}

/// The reply as a frame. Never longer than a frame: an over-long reason is cut.
pub fn reply_frame(reply: &Reply) -> Vec<u8> {
    let reply = match reply {
        Reply::Refused { reason } if reason.len() > 1000 => Reply::Refused { reason: reason.chars().take(300).collect() },
        other => other.clone(),
    };
    serde_json::to_vec(&reply).ok().and_then(|bytes| frame(&bytes)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn soon() -> Instant {
        Instant::now() + Duration::from_millis(200)
    }

    #[test]
    fn a_frame_round_trips() {
        let framed = frame(b"hello").unwrap();
        assert_eq!(framed, [0, 5, b'h', b'e', b'l', b'l', b'o']);
        assert_eq!(read_frame(&mut framed.as_slice(), 16, soon()).unwrap(), b"hello");
    }

    #[test]
    fn refuses_a_frame_over_the_limit_without_reading_the_body() {
        // Announces 60,000 bytes and sends none of them: refused on the length alone.
        assert_eq!(read_frame(&mut [0xEA, 0x60].as_slice(), 1024, soon()), Err(FrameError::TooLarge));
        assert_eq!(frame(&vec![0u8; 70_000]), None);
    }

    #[test]
    fn a_short_frame_is_closed_not_hung() {
        assert_eq!(read_frame(&mut [0, 9, b'x'].as_slice(), 16, soon()), Err(FrameError::Closed));
        assert_eq!(read_frame(&mut [0u8].as_slice(), 16, soon()), Err(FrameError::Closed));
        assert_eq!(read_frame(&mut [].as_slice(), 16, soon()), Err(FrameError::Closed));
    }

    #[test]
    fn a_silent_client_times_out() {
        struct Silent;
        impl Read for Silent {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::WouldBlock.into())
            }
        }
        let started = Instant::now();
        assert_eq!(read_frame(&mut Silent, 16, Instant::now() + Duration::from_millis(60)), Err(FrameError::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_slow_client_that_finishes_in_time_is_served() {
        struct Dribble(Vec<u8>, bool);
        impl Read for Dribble {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                self.1 = !self.1;
                if self.1 || self.0.is_empty() {
                    return Err(io::ErrorKind::WouldBlock.into());
                }
                out[0] = self.0.remove(0);
                Ok(1)
            }
        }
        let mut slow = Dribble(frame(b"abc").unwrap(), false);
        assert_eq!(read_frame(&mut slow, 16, Instant::now() + Duration::from_secs(2)).unwrap(), b"abc");
    }

    #[test]
    fn replies_round_trip_as_json() {
        let ok = Reply::from(&Ok(Outcome::Failed(1603)));
        let bytes = serde_json::to_vec(&ok).unwrap();
        assert_eq!(String::from_utf8(bytes.clone()).unwrap(), r#"{"result":"ran","outcome":{"failed":1603}}"#);
        assert_eq!(serde_json::from_slice::<Reply>(&bytes).unwrap(), ok);
        let no = Reply::from(&Err(Refusal::NotNewer));
        assert!(matches!(&no, Reply::Refused { reason } if reason.contains("not newer")));
        // A decline is told apart from a refusal, so the launcher can fall back to its own route.
        let polite = Reply::from(&Err(Refusal::Declined("installed outside Program Files".into())));
        assert_eq!(serde_json::to_string(&polite).unwrap(), r#"{"result":"declined","reason":"installed outside Program Files"}"#);
    }

    #[test]
    fn an_oversize_reason_still_fits_in_a_frame() {
        let framed = reply_frame(&Reply::Refused { reason: "x".repeat(100_000) });
        assert!(!framed.is_empty() && framed.len() < 2048);
    }
}
