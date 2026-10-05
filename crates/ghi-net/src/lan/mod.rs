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
//!
//! W0 (slice 15-A): the API and its guards are fixed; sockets and mDNS bodies
//! return [`LanError::NotYet`] until slice 15-E.

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::is_lan;

#[cfg(feature = "mdns")]
mod mdns;
#[cfg(feature = "mdns")]
pub use mdns::{MdnsAdvertiser, MdnsDiscovery};

/// Most discovery candidates kept and tried per scan (T12).
pub const MAX_CANDIDATES: usize = 8;
/// TCP connect timeout.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// Not implemented yet (W0 stub).
    NotYet(&'static str),
}

impl fmt::Display for LanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LanError::NotLan(ip) => write!(f, "{ip} is not a LAN address"),
            LanError::Io(e) => write!(f, "network: {e}"),
            LanError::Timeout => f.write_str("timed out"),
            LanError::NotYet(what) => write!(f, "not implemented yet: {what}"),
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

/// The listening side (the hub): one socket per LAN address.
#[derive(Debug)]
pub struct Listener {
    sockets: Vec<TcpListener>,
}

impl Listener {
    /// Binds a socket on each address, all of which must be [`is_lan`]
    /// (`0.0.0.0` and `::` are refused too). Counted in [`listeners_open`]
    /// until dropped.
    pub fn bind(addrs: &[IpAddr]) -> Result<Listener, LanError> {
        if let Some(bad) = addrs.iter().find(|ip| !is_lan(**ip)) {
            return Err(LanError::NotLan(*bad));
        }
        Err(LanError::NotYet("lan::Listener::bind"))
    }

    /// Waits for the next peer. A peer whose address is not LAN is closed
    /// before a byte is read; unknown-key rate limits and the session cap
    /// apply here.
    pub fn accept(&self) -> Result<(LanStream, SocketAddr), LanError> {
        Err(LanError::NotYet("lan::Listener::accept"))
    }

    /// The addresses (with ports) this listener is bound to.
    pub fn local_addrs(&self) -> Vec<SocketAddr> {
        self.sockets
            .iter()
            .filter_map(|s| s.local_addr().ok())
            .collect()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        LISTENERS.fetch_sub(self.sockets.len() as u64, Ordering::SeqCst);
    }
}

/// Connects to a LAN peer (the phone, to the desktop). Refuses an address that
/// is not [`is_lan`] before any syscall.
pub fn connect(addr: SocketAddr, _timeout: Duration) -> Result<LanStream, LanError> {
    if !is_lan(addr.ip()) {
        return Err(LanError::NotLan(addr.ip()));
    }
    Err(LanError::NotYet("lan::connect"))
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

    fn sa(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn bind_refuses_anything_but_lan_addresses() {
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
                matches!(Listener::bind(&[ip]), Err(LanError::NotLan(_))),
                "{ip}"
            );
        }
        // One bad address refuses the lot.
        let mixed = ["192.168.1.2".parse().unwrap(), "127.0.0.1".parse().unwrap()];
        assert!(matches!(Listener::bind(&mixed), Err(LanError::NotLan(_))));
        assert_eq!(listeners_open(), 0);
    }

    #[test]
    fn connect_refuses_non_lan_before_any_syscall() {
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
