// SPDX-License-Identifier: Apache-2.0
//! In-memory stand-ins for the network, so sessions, merges, leases and the
//! MITM tests run with no sockets (there is no loopback bypass in sync, not
//! even for tests):
//! - [`MemDuplex`]: a pair of connected [`Transport`]s (messages).
//! - [`mem_pipe`]: a pair of connected [`ByteStream`]s (bytes), to run the
//!   Noise handshake over, or to put a relaying proxy in between.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::transport::{ByteStream, Transport};
use crate::{Result, SyncError};

/// One end of an in-memory message channel. Dropping an end closes the other
/// (its `recv` returns [`SyncError::Closed`] once the queue is empty).
#[derive(Debug)]
pub struct MemDuplex {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    peer_static: [u8; 32],
    timeout: Option<Duration>,
}

impl MemDuplex {
    /// Two connected ends. Each reports the other's static key from
    /// [`Transport::peer_static`]: `a`'s end sees `b_static` and vice versa.
    pub fn pair(a_static: [u8; 32], b_static: [u8; 32]) -> (MemDuplex, MemDuplex) {
        let (a_tx, b_rx) = mpsc::channel();
        let (b_tx, a_rx) = mpsc::channel();
        (
            MemDuplex {
                tx: a_tx,
                rx: a_rx,
                peer_static: b_static,
                timeout: None,
            },
            MemDuplex {
                tx: b_tx,
                rx: b_rx,
                peer_static: a_static,
                timeout: None,
            },
        )
    }
}

impl Transport for MemDuplex {
    fn send(&mut self, msg: &[u8]) -> Result<()> {
        self.tx.send(msg.to_vec()).map_err(|_| SyncError::Closed)
    }

    fn recv(&mut self) -> Result<Vec<u8>> {
        match self.timeout {
            None => self.rx.recv().map_err(|_| SyncError::Closed),
            Some(t) => self.rx.recv_timeout(t).map_err(|e| match e {
                RecvTimeoutError::Timeout => SyncError::Timeout,
                RecvTimeoutError::Disconnected => SyncError::Closed,
            }),
        }
    }

    fn peer_static(&self) -> [u8; 32] {
        self.peer_static
    }

    fn set_recv_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.timeout = timeout;
        Ok(())
    }
}

#[derive(Debug, Default)]
struct PipeState {
    buf: VecDeque<u8>,
    /// The writing end was dropped: reads drain the buffer, then return 0.
    closed: bool,
}

#[derive(Debug, Default)]
struct Pipe {
    state: Mutex<PipeState>,
    ready: Condvar,
}

impl Pipe {
    fn state(&self) -> std::sync::MutexGuard<'_, PipeState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// One end of an in-memory byte stream.
#[derive(Debug)]
pub struct MemStream {
    rx: Arc<Pipe>,
    tx: Arc<Pipe>,
    timeout: Option<Duration>,
}

/// Two connected byte streams.
pub fn mem_pipe() -> (MemStream, MemStream) {
    let (ab, ba) = (Arc::new(Pipe::default()), Arc::new(Pipe::default()));
    (
        MemStream {
            rx: ba.clone(),
            tx: ab.clone(),
            timeout: None,
        },
        MemStream {
            rx: ab,
            tx: ba,
            timeout: None,
        },
    )
}

impl Read for MemStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let mut st = self.rx.state();
        while st.buf.is_empty() {
            if st.closed {
                return Ok(0);
            }
            st = match self.timeout {
                None => self.rx.ready.wait(st).unwrap_or_else(|p| p.into_inner()),
                Some(t) => {
                    let (guard, res) = self
                        .rx
                        .ready
                        .wait_timeout(st, t)
                        .unwrap_or_else(|p| p.into_inner());
                    if res.timed_out() && guard.buf.is_empty() && !guard.closed {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                    guard
                }
            };
        }
        let n = out.len().min(st.buf.len());
        for (slot, byte) in out.iter_mut().zip(st.buf.drain(..n)) {
            *slot = byte;
        }
        Ok(n)
    }
}

impl Write for MemStream {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut st = self.tx.state();
        if st.closed {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        st.buf.extend(data);
        self.tx.ready.notify_all();
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for MemStream {
    fn drop(&mut self) {
        // The peer's reads end once drained; its writes now fail.
        self.tx.state().closed = true;
        self.tx.ready.notify_all();
        self.rx.state().closed = true;
        self.rx.ready.notify_all();
    }
}

impl ByteStream for MemStream {
    fn set_io_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.timeout = timeout;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplex_carries_messages_both_ways_and_reports_the_peer_key() {
        let (mut a, mut b) = MemDuplex::pair([1; 32], [2; 32]);
        assert_eq!(a.peer_static(), [2; 32]);
        assert_eq!(b.peer_static(), [1; 32]);
        a.send(b"one").unwrap();
        a.send(b"two").unwrap();
        b.send(b"back").unwrap();
        assert_eq!(b.recv().unwrap(), b"one");
        assert_eq!(b.recv().unwrap(), b"two");
        assert_eq!(a.recv().unwrap(), b"back");
    }

    #[test]
    fn duplex_close_and_timeout() {
        let (mut a, mut b) = MemDuplex::pair([1; 32], [2; 32]);
        b.set_recv_timeout(Some(Duration::from_millis(20))).unwrap();
        assert!(matches!(b.recv(), Err(SyncError::Timeout)));
        a.send(b"last").unwrap();
        drop(a);
        assert_eq!(
            b.recv().unwrap(),
            b"last",
            "queued messages survive the close"
        );
        assert!(matches!(b.recv(), Err(SyncError::Closed)));
        assert!(matches!(b.send(b"x"), Err(SyncError::Closed)));
    }

    #[test]
    fn pipe_carries_bytes_and_ends_on_drop() {
        let (mut a, mut b) = mem_pipe();
        a.write_all(b"hello").unwrap();
        let mut got = [0u8; 3];
        b.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"hel");
        let t = std::thread::spawn(move || {
            b.write_all(b"pong").unwrap();
            let mut rest = Vec::new();
            b.read_to_end(&mut rest).unwrap();
            rest
        });
        let mut pong = [0u8; 4];
        a.read_exact(&mut pong).unwrap();
        assert_eq!(&pong, b"pong");
        a.write_all(b"lo").unwrap();
        drop(a);
        // The unread "lo" of "hello", then the new "lo".
        assert_eq!(t.join().unwrap(), b"lolo");
    }

    #[test]
    fn pipe_write_to_a_dropped_peer_fails_and_reads_time_out() {
        let (mut a, b) = mem_pipe();
        a.set_io_timeout(Some(Duration::from_millis(20))).unwrap();
        let mut one = [0u8; 1];
        assert_eq!(
            a.read(&mut one).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        drop(b);
        assert_eq!(a.write(b"x").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(a.read(&mut one).unwrap(), 0);
    }
}
