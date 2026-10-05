// SPDX-License-Identifier: Apache-2.0
//! LAN transport and discovery for sync (phase 15, doc 07 §4). Every socket
//! and every mDNS packet of sync is here and nowhere else (RT-6).
//!
//! Rules that hold for all of it:
//! - An address is used only if [`crate::is_lan`] says so (RFC 1918 and
//!   fc00::/7). Never loopback, link-local, CGNAT or public, and never
//!   `NetPolicy::permits`, which accepts link-local and `.local`.
//! - [`Listener::bind`] binds one socket per LAN address, never `0.0.0.0`.
//!   [`connect`] refuses a non-LAN peer before any syscall.
//! - Counters are separate from [`crate::connections_opened`], which keeps
//!   meaning "internet egress": [`listeners_open`] counts open listening
//!   sockets, [`lan_connections_opened`] counts outgoing LAN connections.
//! - Discovery is only a hint ([`Discovery`]); identity is proven by the
//!   handshake above this layer.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::is_lan;

mod limits;
pub use limits::{Limits, MAX_SESSIONS, MAX_UNKNOWN_PER_MINUTE, SessionSlot};

#[cfg(feature = "mdns")]
mod mdns;
#[cfg(feature = "mdns")]
pub use mdns::{MdnsAdvertiser, MdnsDiscovery};

/// Most discovery candidates kept and tried per scan (T12).
pub const MAX_CANDIDATES: usize = 8;
/// TCP connect timeout.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Read/write limit `accept` puts on a new stream, for the handshake. The
/// session layer replaces it once the peer is authenticated.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const ACCEPT_POLL: Duration = Duration::from_millis(25);

static LISTENERS: AtomicU64 = AtomicU64::new(0);
static LAN_CONNECTIONS: AtomicU64 = AtomicU64::new(0);

/// Listening sockets open right now (0 when sync is off).
pub fn listeners_open() -> u64 {
    LISTENERS.load(Ordering::SeqCst)
}

/// Outgoing LAN connections opened since the process started. Never counts
/// internet traffic (see [`crate::connections_opened`]).
pub fn lan_connections_opened() -> u64 {
    LAN_CONNECTIONS.load(Ordering::SeqCst)
}

/// The addresses of this computer that sync may use: up, not loopback, and
/// [`is_lan`](crate::is_lan). A listener binds these and mDNS advertises them.
pub fn lan_addrs() -> Vec<IpAddr> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut out: Vec<IpAddr> = ifaces
        .into_iter()
        .filter(|i| !i.is_loopback())
        .map(|i| i.ip())
        .filter(|ip| is_lan(*ip))
        .collect();
    out.sort();
    out.dedup();
    out
}

#[derive(Debug)]
pub enum LanError {
    /// The address is not a private LAN address; nothing was opened.
    NotLan(IpAddr),
    Io(io::Error),
    Timeout,
    /// There is no LAN address to bind to or advertise.
    NoAddress,
    /// mDNS failed to start or to register (the text has no addresses).
    Mdns(String),
}

impl fmt::Display for LanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LanError::NotLan(ip) => write!(f, "{ip} is not a LAN address"),
            LanError::Io(e) => write!(f, "network: {e}"),
            LanError::Timeout => f.write_str("timed out"),
            LanError::NoAddress => f.write_str("no LAN address available"),
            LanError::Mdns(what) => write!(f, "mDNS: {what}"),
        }
    }
}

impl std::error::Error for LanError {}

impl From<io::Error> for LanError {
    fn from(e: io::Error) -> Self {
        LanError::Io(e)
    }
}

/// A connected LAN peer: a plain byte stream (Noise and framing go on top, in
/// `ghi-sync`).
#[derive(Debug)]
pub struct LanStream {
    inner: TcpStream,
    /// Frees a hub session slot when the stream is dropped (accepted streams).
    _slot: Option<SessionSlot>,
}

impl LanStream {
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.inner.peer_addr()
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.inner.set_read_timeout(timeout)
    }

    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.inner.set_write_timeout(timeout)
    }
}

impl Read for LanStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for LanStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// The listening side (the hub): one socket per LAN address, all on one port.
#[derive(Debug)]
pub struct Listener {
    sockets: Vec<(IpAddr, TcpListener)>,
    port: u16,
    limits: Arc<Limits>,
}

fn bind_one(ip: IpAddr, port: u16) -> Result<TcpListener, LanError> {
    let s = TcpListener::bind(SocketAddr::new(ip, port))?;
    s.set_nonblocking(true)?;
    Ok(s)
}

fn check_lan(addrs: &[IpAddr]) -> Result<Vec<IpAddr>, LanError> {
    if let Some(bad) = addrs.iter().find(|ip| !is_lan(**ip)) {
        return Err(LanError::NotLan(*bad));
    }
    let mut out = addrs.to_vec();
    out.sort();
    out.dedup();
    Ok(out)
}

impl Listener {
    /// Binds a socket on each address, all of which must be [`is_lan`]
    /// (`0.0.0.0` and `::` are refused too), on `port` (0 = pick one; the
    /// caller persists [`Listener::port`] and passes it next time). Counted
    /// in [`listeners_open`] until dropped.
    pub fn bind(addrs: &[IpAddr], port: u16) -> Result<Listener, LanError> {
        let addrs = check_lan(addrs)?;
        if addrs.is_empty() {
            return Err(LanError::NoAddress);
        }
        let mut sockets = Vec::with_capacity(addrs.len());
        let mut port = port;
        for ip in addrs {
            let s = bind_one(ip, port)?;
            port = s.local_addr()?.port();
            sockets.push((ip, s));
        }
        LISTENERS.fetch_add(sockets.len() as u64, Ordering::SeqCst);
        Ok(Listener {
            sockets,
            port,
            limits: Limits::new(),
        })
    }

    /// The port every socket listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The limits this listener enforces; the session layer calls
    /// [`Limits::report_handshake_failure`] on it when a handshake fails.
    pub fn limits(&self) -> Arc<Limits> {
        Arc::clone(&self.limits)
    }

    /// Makes the sockets match `current` (usually [`lan_addrs`]): sockets of
    /// vanished addresses close, new ones bind on the same port, the others
    /// stay. Returns whether anything changed; on error nothing changed.
    pub fn rebind(&mut self, current: &[IpAddr]) -> Result<bool, LanError> {
        let want = check_lan(current)?;
        let have: Vec<IpAddr> = self.sockets.iter().map(|(ip, _)| *ip).collect();
        if want == have {
            return Ok(false);
        }
        // Bind the new addresses first: on failure nothing has changed.
        let mut added = Vec::new();
        for ip in want.iter().filter(|ip| !have.contains(ip)) {
            added.push((*ip, bind_one(*ip, self.port)?));
        }
        let before = self.sockets.len() as u64;
        self.sockets.retain(|(ip, _)| want.contains(ip));
        self.sockets.extend(added);
        self.sockets.sort_by_key(|(ip, _)| *ip);
        let after = self.sockets.len() as u64;
        LISTENERS.fetch_add(after, Ordering::SeqCst);
        LISTENERS.fetch_sub(before, Ordering::SeqCst);
        Ok(true)
    }

    /// Waits up to `timeout` for the next acceptable peer. A peer whose
    /// address is not LAN, one that is banned, and any peer past the session
    /// cap are closed before a byte is read. The stream comes with
    /// [`HANDSHAKE_TIMEOUT`] as read and write limit.
    pub fn accept_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Option<(LanStream, SocketAddr)>, LanError> {
        let deadline = Instant::now() + timeout;
        loop {
            for (_, sock) in &self.sockets {
                match sock.accept() {
                    Ok((stream, peer)) => {
                        if let Some(ok) = self.admit(stream, peer) {
                            return Ok(Some(ok));
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    // A peer that vanished between SYN and accept.
                    Err(e) if e.kind() == io::ErrorKind::ConnectionAborted => {}
                    Err(e) => return Err(e.into()),
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            std::thread::sleep(ACCEPT_POLL.min(deadline - now));
        }
    }

    /// Like [`Listener::accept_timeout`], without a limit.
    pub fn accept(&self) -> Result<(LanStream, SocketAddr), LanError> {
        loop {
            if let Some(got) = self.accept_timeout(Duration::from_secs(3600))? {
                return Ok(got);
            }
        }
    }

    fn admit(&self, stream: TcpStream, peer: SocketAddr) -> Option<(LanStream, SocketAddr)> {
        if !is_lan(peer.ip()) {
            log::warn!("lan: dropped a non-LAN peer");
            return None;
        }
        if self.limits.is_banned(peer.ip()) {
            log::info!("lan: dropped a banned peer");
            return None;
        }
        let Some(slot) = self.limits.try_slot() else {
            log::info!("lan: session limit reached, dropped a peer");
            return None;
        };
        // BSD sockets inherit O_NONBLOCK from the listener.
        let ready = stream.set_nonblocking(false).is_ok()
            && stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).is_ok()
            && stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT)).is_ok();
        if !ready {
            return None;
        }
        let _ = stream.set_nodelay(true);
        Some((
            LanStream {
                inner: stream,
                _slot: Some(slot),
            },
            peer,
        ))
    }

    /// The addresses (with ports) this listener is bound to.
    pub fn local_addrs(&self) -> Vec<SocketAddr> {
        self.sockets
            .iter()
            .filter_map(|(_, s)| s.local_addr().ok())
            .collect()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        LISTENERS.fetch_sub(self.sockets.len() as u64, Ordering::SeqCst);
    }
}

/// Connects to a LAN peer (the phone, to the desktop). Refuses an address that
/// is not [`is_lan`] before any syscall. Counted in
/// [`lan_connections_opened`] once connected.
pub fn connect(addr: SocketAddr, timeout: Duration) -> Result<LanStream, LanError> {
    if !is_lan(addr.ip()) {
        return Err(LanError::NotLan(addr.ip()));
    }
    let stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => LanError::Timeout,
        _ => LanError::Io(e),
    })?;
    LAN_CONNECTIONS.fetch_add(1, Ordering::SeqCst);
    let _ = stream.set_nodelay(true);
    Ok(LanStream {
        inner: stream,
        _slot: None,
    })
}

/// Where the next connection attempt might go. Hints only: the handshake
/// decides who is who.
pub trait Discovery: Send + Sync {
    /// LAN candidates, most promising first, at most [`MAX_CANDIDATES`].
    fn candidates(&self) -> Vec<SocketAddr>;
}

fn keep_lan(addrs: impl IntoIterator<Item = SocketAddr>) -> Vec<SocketAddr> {
    let mut out: Vec<SocketAddr> = Vec::new();
    for a in addrs {
        if is_lan(a.ip()) && !out.contains(&a) && out.len() < MAX_CANDIDATES {
            out.push(a);
        }
    }
    out
}

/// A fixed list: the QR or `PairAccept` addresses, the last one that worked.
#[derive(Debug, Clone, Default)]
pub struct StaticDiscovery {
    addrs: Vec<SocketAddr>,
}

impl StaticDiscovery {
    /// Keeps the LAN addresses of `addrs` (the rest are dropped).
    pub fn new(addrs: impl IntoIterator<Item = SocketAddr>) -> Self {
        Self {
            addrs: keep_lan(addrs),
        }
    }
}

impl Discovery for StaticDiscovery {
    fn candidates(&self) -> Vec<SocketAddr> {
        self.addrs.clone()
    }
}

/// Candidates pushed in by someone else: the iOS app feeds it from
/// `NWBrowser` over the C ABI.
#[derive(Debug, Default)]
pub struct PushedDiscovery {
    addrs: Mutex<Vec<SocketAddr>>,
}

impl PushedDiscovery {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the candidates with the LAN ones of `addrs`.
    pub fn set(&self, addrs: impl IntoIterator<Item = SocketAddr>) {
        let kept = keep_lan(addrs);
        *self.addrs.lock().unwrap_or_else(|p| p.into_inner()) = kept;
    }
}

impl Discovery for PushedDiscovery {
    fn candidates(&self) -> Vec<SocketAddr> {
        self.addrs.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Socket tests share the global counters: one at a time.
    static SOCKETS: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SOCKETS.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The private address the socket tests use (there is no loopback bypass,
    /// not even here): `GHI_SYNC_LAN_IP`, which must pass `is_lan`.
    pub(crate) fn test_lan_ip() -> Option<IpAddr> {
        let ip = std::env::var("GHI_SYNC_LAN_IP")
            .ok()
            .and_then(|v| v.trim().parse::<IpAddr>().ok())
            .filter(|ip| is_lan(*ip));
        if ip.is_none() {
            eprintln!("SKIPPED: set GHI_SYNC_LAN_IP to a private LAN address to run this test");
        }
        ip
    }

    fn sa(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn bind_refuses_anything_but_lan_addresses() {
        let _g = serial();
        for ip in [
            "127.0.0.1",
            "0.0.0.0",
            "169.254.1.1",
            "100.64.0.1",
            "8.8.8.8",
            "::1",
            "::",
            "fe80::1",
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(
                matches!(Listener::bind(&[ip], 0), Err(LanError::NotLan(_))),
                "{ip}"
            );
        }
        // One bad address refuses the lot.
        let mixed = ["192.168.1.2".parse().unwrap(), "127.0.0.1".parse().unwrap()];
        assert!(matches!(
            Listener::bind(&mixed, 0),
            Err(LanError::NotLan(_))
        ));
        assert_eq!(listeners_open(), 0);
    }

    #[test]
    fn connect_refuses_non_lan_before_any_syscall() {
        let _g = serial();
        let before = lan_connections_opened();
        for a in [
            "127.0.0.1:1",
            "169.254.1.1:1",
            "100.64.0.1:1",
            "1.1.1.1:1",
            "[fe80::1]:1",
        ] {
            let r = connect(sa(a), CONNECT_TIMEOUT);
            assert!(matches!(r, Err(LanError::NotLan(_))), "{a}");
        }
        assert_eq!(lan_connections_opened(), before);
    }

    #[test]
    fn bind_with_no_address_is_an_error() {
        assert!(matches!(Listener::bind(&[], 0), Err(LanError::NoAddress)));
    }

    #[test]
    fn accept_connect_count_and_close() {
        let _g = serial();
        let Some(ip) = test_lan_ip() else { return };
        let (open0, conns0) = (listeners_open(), lan_connections_opened());
        let l = Listener::bind(&[ip], 0).unwrap();
        assert_eq!(listeners_open(), open0 + 1);
        let addr = SocketAddr::new(ip, l.port());
        assert_eq!(l.local_addrs(), vec![addr]);
        assert!(
            l.accept_timeout(Duration::from_millis(50))
                .unwrap()
                .is_none()
        );

        let mut c = connect(addr, CONNECT_TIMEOUT).unwrap();
        assert_eq!(lan_connections_opened(), conns0 + 1);
        let (mut s, peer) = l.accept_timeout(Duration::from_secs(2)).unwrap().unwrap();
        assert_eq!(peer.ip(), ip);
        assert_eq!(l.limits().sessions(), 1);
        c.write_all(b"ping").unwrap();
        let mut got = [0u8; 4];
        s.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"ping");
        drop(s);
        assert_eq!(l.limits().sessions(), 0);

        // The port survives a rebind to nothing and back.
        let port = l.port();
        let mut l = l;
        assert!(!l.rebind(&[ip]).unwrap());
        assert!(l.rebind(&[]).unwrap());
        assert_eq!(listeners_open(), open0);
        assert!(l.rebind(&[ip]).unwrap());
        assert_eq!((l.port(), listeners_open()), (port, open0 + 1));
        drop(l);
        assert_eq!(listeners_open(), open0);
        // Rebinding the persisted port works after the drop.
        let again = Listener::bind(&[ip], port).unwrap();
        assert_eq!(again.port(), port);
    }

    #[test]
    fn banned_peers_and_a_full_house_are_dropped_unread() {
        let _g = serial();
        let Some(ip) = test_lan_ip() else { return };
        let l = Listener::bind(&[ip], 0).unwrap();
        let addr = SocketAddr::new(ip, l.port());

        // Four sessions fit; the fifth is closed without a byte.
        let mut held = Vec::new();
        let mut clients = Vec::new();
        for _ in 0..MAX_SESSIONS {
            clients.push(connect(addr, CONNECT_TIMEOUT).unwrap());
            held.push(l.accept_timeout(Duration::from_secs(2)).unwrap().unwrap());
        }
        let mut fifth = connect(addr, CONNECT_TIMEOUT).unwrap();
        assert!(
            l.accept_timeout(Duration::from_millis(300))
                .unwrap()
                .is_none()
        );
        fifth
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        assert_eq!(fifth.read(&mut [0u8; 1]).unwrap_or(0), 0, "closed");
        drop(held);

        // Ten failures from one address ban it.
        for _ in 0..MAX_UNKNOWN_PER_MINUTE {
            l.limits().report_handshake_failure(ip);
        }
        let mut banned = connect(addr, CONNECT_TIMEOUT).unwrap();
        assert!(
            l.accept_timeout(Duration::from_millis(300))
                .unwrap()
                .is_none()
        );
        banned
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        assert_eq!(banned.read(&mut [0u8; 1]).unwrap_or(0), 0, "closed");
    }

    #[test]
    fn lan_addrs_are_all_lan() {
        assert!(lan_addrs().into_iter().all(is_lan));
    }

    #[test]
    fn static_discovery_keeps_only_lan_and_caps() {
        let many: Vec<SocketAddr> = (1..=20).map(|i| sa(&format!("10.0.0.{i}:7"))).collect();
        let d = StaticDiscovery::new(
            [sa("127.0.0.1:7"), sa("8.8.8.8:7"), sa("169.254.0.9:7")]
                .into_iter()
                .chain(many),
        );
        let got = d.candidates();
        assert_eq!(got.len(), MAX_CANDIDATES);
        assert!(got.iter().all(|a| is_lan(a.ip())));
    }

    #[test]
    fn pushed_discovery_replaces_and_filters() {
        let d = PushedDiscovery::new();
        assert!(d.candidates().is_empty());
        d.set([sa("192.168.1.5:9"), sa("192.168.1.5:9"), sa("1.2.3.4:9")]);
        assert_eq!(d.candidates(), vec![sa("192.168.1.5:9")]);
        d.set([]);
        assert!(d.candidates().is_empty());
    }
}
