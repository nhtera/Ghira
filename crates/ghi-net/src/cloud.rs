// SPDX-License-Identifier: Apache-2.0
//! Cloud requests: a [`CloudGrant`] binds one exact request, and [`send`] is
//! the only way to put it on the wire.
//!
//! A grant is minted after the user confirmed the send preview. It binds
//! (scheme, host, port, path and query, SHA-256 of the body): `send` recomputes
//! all of it before it opens a socket, so what leaves the device is the
//! previewed bytes and nothing else. Hardening (RT-6):
//! - refused under strict offline and when any involved meeting is cloud-locked
//!   or sensitive;
//! - https only; no proxy (the environment's `HTTPS_PROXY` is ignored); any
//!   3xx is an error (a redirect would carry the key elsewhere);
//! - the host is resolved once, through a resolver that keeps only public
//!   addresses and hands exactly those to the connector (no second lookup, so
//!   no DNS rebinding); a host that is itself a LAN host (private IP literal or
//!   `.local`) may use private addresses; loopback is never allowed;
//! - TTL 10 minutes, at most 3 attempts; only 429, 5xx and transport failures
//!   may be retried, with the identical bytes;
//! - errors never carry headers or request bodies.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use ureq::config::Config;
use ureq::http::Uri;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{
    ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport,
};

use crate::ip;
use crate::secret::Headers;
use crate::{Destination, NetPolicy, classify};

/// How long a minted grant stays valid.
pub const GRANT_TTL: Duration = Duration::from_secs(10 * 60);
/// Sends one grant allows (the first plus retries).
pub const MAX_ATTEMPTS: u8 = 3;
/// Time for the whole request, including a slow generation.
const TIMEOUT_GLOBAL: Duration = Duration::from_secs(300);
const TIMEOUT_CONNECT: Duration = Duration::from_secs(15);
/// Largest response body read.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

static CONNECTIONS: AtomicU64 = AtomicU64::new(0);

/// Connection attempts this process has made through `ghi-net` (every call of
/// the connector, successful or not). Golden tests assert it stays 0 for local
/// and denied runs.
pub fn connections_opened() -> u64 {
    CONNECTIONS.load(Ordering::SeqCst)
}

/// Hex SHA-256, what the user confirms in the send preview.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[derive(Debug)]
pub enum NetError {
    /// Refused by policy: strict offline, cloud-locked or sensitive meeting,
    /// not https, a private or loopback host.
    Denied(String),
    /// The request does not match what the grant was minted for.
    Mismatch(&'static str),
    Expired,
    /// The grant's attempts are used up, or it is spent (answered, or failed
    /// in a way that may not be retried).
    Exhausted,
    /// The host resolved to no allowed address.
    Blocked(String),
    /// The server answered 3xx; redirects are never followed.
    Redirect(u16),
    /// Connect, TLS, timeout or I/O failure (no request data in the text).
    Transport(String),
    /// The response body exceeded the limit.
    TooLarge,
}

impl NetError {
    /// Whether the same grant may send the same bytes again.
    pub fn is_retryable(&self) -> bool {
        matches!(self, NetError::Transport(_))
    }
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Denied(d) => write!(f, "not allowed: {d}"),
            NetError::Mismatch(what) => {
                write!(f, "request does not match the confirmed send ({what})")
            }
            NetError::Expired => f.write_str("the send confirmation expired; preview again"),
            NetError::Exhausted => f.write_str("no attempts left on this send confirmation"),
            NetError::Blocked(host) => {
                write!(f, "{host} resolves only to non-public addresses; refused")
            }
            NetError::Redirect(code) => write!(f, "the server redirected ({code}); refused"),
            NetError::Transport(d) => write!(f, "transport: {d}"),
            NetError::TooLarge => f.write_str("the response was too large"),
        }
    }
}

impl std::error::Error for NetError {}

/// What one meeting contributes to a cloud-send decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeetingGate {
    /// The meeting was marked never-cloud.
    pub cloud_locked: bool,
    /// The meeting was marked sensitive.
    pub sensitive: bool,
}

/// Test-only relaxations (plain http to a loopback mock server). Only a
/// `#[cfg(test)]` constructor can turn them on, so non-test builds have no way
/// to reach loopback or use plain http.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Relax {
    loopback_http: bool,
}

impl Relax {
    pub(crate) const fn none() -> Relax {
        Relax {
            loopback_http: false,
        }
    }

    #[cfg(test)]
    pub(crate) const fn loopback() -> Relax {
        Relax {
            loopback_http: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    host: String,
    port: u16,
    path_and_query: String,
}

fn parse_target(url: &str, relax: Relax) -> Result<Target, NetError> {
    let denied = |d: &str| NetError::Denied(d.to_owned());
    let uri: Uri = url.parse().map_err(|_| denied("not a valid URL"))?;
    let scheme = uri.scheme_str().unwrap_or("");
    let default_port = match scheme {
        "https" => 443,
        "http" if relax.loopback_http => 80,
        _ => return Err(denied("only https URLs are allowed")),
    };
    let authority = uri.authority().ok_or_else(|| denied("no host in URL"))?;
    if authority.as_str().contains('@') {
        return Err(denied("credentials in the URL are not allowed"));
    }
    let host = uri
        .host()
        .unwrap_or("")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host.is_empty() {
        return Err(denied("no host in URL"));
    }
    let path_and_query = uri.path_and_query().map_or("/", |p| p.as_str()).to_owned();
    Ok(Target {
        host,
        port: uri.port_u16().unwrap_or(default_port),
        path_and_query,
    })
}

fn is_lan_host(host: &str) -> bool {
    classify(host, &[]) == Destination::PrivateLan
}

/// Permission to send exactly one request. See the module docs.
#[derive(Debug)]
pub struct CloudGrant {
    target: Target,
    body_sha256: [u8; 32],
    expires_at: Instant,
    attempts_left: u8,
    /// The last send got an answer or an error that may not be retried.
    spent: bool,
    allow_lan: bool,
    relax: Relax,
}

impl CloudGrant {
    /// Mints a grant for `url` and the exact `body`. `gates` is one entry per
    /// meeting whose text is in the body (at least one).
    pub fn mint(
        policy: NetPolicy,
        url: &str,
        body: &[u8],
        gates: &[MeetingGate],
    ) -> Result<CloudGrant, NetError> {
        CloudGrant::mint_inner(Relax::none(), policy, url, body, gates)
    }

    pub(crate) fn mint_inner(
        relax: Relax,
        policy: NetPolicy,
        url: &str,
        body: &[u8],
        gates: &[MeetingGate],
    ) -> Result<CloudGrant, NetError> {
        if policy == NetPolicy::StrictOffline {
            return Err(NetError::Denied("strict offline is on".into()));
        }
        if gates.is_empty() {
            return Err(NetError::Denied("no meeting to send".into()));
        }
        if gates.iter().any(|g| g.cloud_locked) {
            return Err(NetError::Denied(
                "a meeting is locked against cloud AI".into(),
            ));
        }
        if gates.iter().any(|g| g.sensitive) {
            return Err(NetError::Denied("a meeting is marked sensitive".into()));
        }
        let target = parse_target(url, relax)?;
        let allow_lan = is_lan_host(&target.host);
        if let Ok(ip) = target
            .host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            && !addr_allowed(ip, allow_lan, relax)
        {
            return Err(NetError::Denied("the host is not a public address".into()));
        }
        Ok(CloudGrant {
            target,
            body_sha256: Sha256::digest(body).into(),
            expires_at: Instant::now() + GRANT_TTL,
            attempts_left: MAX_ATTEMPTS,
            spent: false,
            allow_lan,
            relax,
        })
    }

    pub fn host(&self) -> &str {
        &self.target.host
    }

    pub fn attempts_left(&self) -> u8 {
        self.attempts_left
    }

    fn check(&self, url: &str, body: &[u8]) -> Result<(), NetError> {
        if Instant::now() >= self.expires_at {
            return Err(NetError::Expired);
        }
        if self.spent || self.attempts_left == 0 {
            return Err(NetError::Exhausted);
        }
        let t = parse_target(url, self.relax).map_err(|_| NetError::Mismatch("url"))?;
        if t.host != self.target.host {
            return Err(NetError::Mismatch("host"));
        }
        if t.port != self.target.port {
            return Err(NetError::Mismatch("port"));
        }
        if t.path_and_query != self.target.path_and_query {
            return Err(NetError::Mismatch("path"));
        }
        let digest: [u8; 32] = Sha256::digest(body).into();
        if digest != self.body_sha256 {
            return Err(NetError::Mismatch("body"));
        }
        Ok(())
    }
}

/// A completed HTTP exchange (any status except 3xx).
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// POSTs `body` to `url` if the grant allows it. Checks the binding before any
/// socket is opened; one call uses one attempt. A 429, 5xx or transport error
/// leaves the grant usable for an identical retry; any other answer spends it.
pub fn send(
    grant: &mut CloudGrant,
    url: &str,
    headers: &Headers,
    body: &[u8],
) -> Result<HttpResponse, NetError> {
    grant.check(url, body)?;
    grant.attempts_left -= 1;
    let result = exchange(grant, url, headers, body);
    grant.spent = match &result {
        Ok(r) => !(r.status == 429 || r.status >= 500),
        Err(e) => !e.is_retryable(),
    };
    result
}

fn exchange(
    grant: &CloudGrant,
    url: &str,
    headers: &Headers,
    body: &[u8],
) -> Result<HttpResponse, NetError> {
    let blocked = Arc::new(AtomicBool::new(false));
    let resolver = VettedResolver {
        allow_lan: grant.allow_lan,
        relax: grant.relax,
        blocked: blocked.clone(),
    };
    let agent = ureq::Agent::with_parts(client_config(grant.relax), CountingConnector, resolver);
    let mut req = agent.post(url);
    for (name, value) in headers.iter() {
        req = req.header(name, value.expose());
    }
    let transport = |e: ureq::Error| {
        if blocked.load(Ordering::SeqCst) {
            NetError::Blocked(grant.target.host.clone())
        } else if matches!(e, ureq::Error::BodyExceedsLimit(_)) {
            NetError::TooLarge
        } else {
            NetError::Transport(e.to_string())
        }
    };
    let mut resp = req.send(body).map_err(transport)?;
    let status = resp.status().as_u16();
    if (300..400).contains(&status) {
        return Err(NetError::Redirect(status));
    }
    let body = resp
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(transport)?;
    Ok(HttpResponse { status, body })
}

fn client_config(relax: Relax) -> Config {
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    Config::builder()
        // Never the ambient HTTP(S)_PROXY / ALL_PROXY.
        .proxy(None)
        .max_redirects(0)
        .https_only(!relax.loopback_http)
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT_GLOBAL))
        .timeout_connect(Some(TIMEOUT_CONNECT))
        .tls_config(tls)
        .user_agent(concat!("ghira/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn addr_allowed(ip: std::net::IpAddr, allow_lan: bool, relax: Relax) -> bool {
    ip::is_public(ip) || (allow_lan && ip::is_lan(ip)) || (relax.loopback_http && ip.is_loopback())
}

/// Keeps only the addresses a grant may connect to.
fn vet_addrs(addrs: &[SocketAddr], allow_lan: bool, relax: Relax) -> Vec<SocketAddr> {
    addrs
        .iter()
        .copied()
        .filter(|a| addr_allowed(a.ip(), allow_lan, relax))
        .collect()
}

/// Resolves once and returns only vetted addresses; the connector connects to
/// exactly these, so the name is never looked up a second time.
#[derive(Debug)]
struct VettedResolver {
    allow_lan: bool,
    relax: Relax,
    blocked: Arc<AtomicBool>,
}

impl Resolver for VettedResolver {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let all = DefaultResolver::default().resolve(uri, config, timeout)?;
        let all: Vec<SocketAddr> = all.iter().copied().collect();
        let kept = vet_addrs(&all, self.allow_lan, self.relax);
        if kept.is_empty() {
            self.blocked.store(true, Ordering::SeqCst);
            return Err(ureq::Error::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "no allowed address",
            )));
        }
        let mut out = self.empty();
        for a in kept {
            out.push(a);
        }
        Ok(out)
    }
}

/// The default TCP + TLS chain, counting every attempt.
#[derive(Debug)]
struct CountingConnector;

impl Connector<()> for CountingConnector {
    type Out = Box<dyn Transport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        CONNECTIONS.fetch_add(1, Ordering::SeqCst);
        DefaultConnector::new().connect(details, chained)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    const OPEN: MeetingGate = MeetingGate {
        cloud_locked: false,
        sensitive: false,
    };
    /// Tests that read the connection counter or connect run one at a time.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    const URL: &str = "https://api.example.com/v1/chat/completions";

    fn mint(url: &str, body: &[u8]) -> Result<CloudGrant, NetError> {
        CloudGrant::mint(NetPolicy::Default, url, body, &[OPEN])
    }

    #[test]
    fn mint_refuses_policy_and_gates() {
        let m = |p, gates: &[MeetingGate]| CloudGrant::mint(p, URL, b"x", gates);
        assert!(m(NetPolicy::Default, &[OPEN]).is_ok());
        assert!(matches!(
            m(NetPolicy::StrictOffline, &[OPEN]),
            Err(NetError::Denied(_))
        ));
        assert!(matches!(
            m(NetPolicy::Default, &[]),
            Err(NetError::Denied(_))
        ));
        let locked = MeetingGate {
            cloud_locked: true,
            sensitive: false,
        };
        let sens = MeetingGate {
            cloud_locked: false,
            sensitive: true,
        };
        assert!(matches!(
            m(NetPolicy::Default, &[OPEN, locked]),
            Err(NetError::Denied(_))
        ));
        assert!(matches!(
            m(NetPolicy::Default, &[sens, OPEN]),
            Err(NetError::Denied(_))
        ));
    }

    #[test]
    fn mint_refuses_bad_urls() {
        for url in [
            "http://api.example.com/x",
            "ftp://api.example.com/x",
            "api.example.com/x",
            "https://user:pw@api.example.com/x",
            "https:///x",
            "https://127.0.0.1/x",
            "https://[::1]/x",
            "https://169.254.169.254/latest",
            "https://0.0.0.0/x",
            "https://[::ffff:127.0.0.1]/x",
        ] {
            assert!(matches!(mint(url, b"x"), Err(NetError::Denied(_))), "{url}");
        }
        // A LAN host may be private; loopback never.
        assert!(mint("https://192.168.1.20:8443/v1", b"x").is_ok());
        assert!(mint("https://ghi-mac.local/v1", b"x").is_ok());
        assert!(mint("https://127.0.0.1/v1", b"x").is_err());
    }

    #[test]
    fn check_binds_host_port_path_and_body() {
        let g = mint(URL, b"body").unwrap();
        assert!(g.check(URL, b"body").is_ok());
        assert!(
            g.check("HTTPS://API.EXAMPLE.COM./v1/chat/completions", b"body")
                .is_ok()
        );
        let mismatch = |url: &str, body: &[u8], what: &str| match g.check(url, body) {
            Err(NetError::Mismatch(w)) => assert_eq!(w, what, "{url}"),
            other => panic!("{url}: {other:?}"),
        };
        mismatch(
            "https://evil.example.com/v1/chat/completions",
            b"body",
            "host",
        );
        mismatch(
            "https://api.example.com:8443/v1/chat/completions",
            b"body",
            "port",
        );
        mismatch("https://api.example.com/v1/other", b"body", "path");
        mismatch(
            "https://api.example.com/v1/chat/completions?x=1",
            b"body",
            "path",
        );
        mismatch(URL, b"body!", "body");
        mismatch(URL, b"", "body");
        mismatch("http://api.example.com/v1/chat/completions", b"body", "url");
    }

    #[test]
    fn expired_grant_is_refused() {
        let _serial = serial();
        let mut g = mint(URL, b"b").unwrap();
        g.expires_at = Instant::now() - Duration::from_secs(1);
        assert!(matches!(g.check(URL, b"b"), Err(NetError::Expired)));
        let before = connections_opened();
        let r = send(&mut g, URL, &Headers::new(), b"b");
        assert!(matches!(r, Err(NetError::Expired)));
        assert_eq!(connections_opened(), before);
    }

    #[test]
    fn mismatch_opens_no_socket_and_keeps_attempts() {
        let _serial = serial();
        let mut g = mint(URL, b"b").unwrap();
        let before = connections_opened();
        let r = send(&mut g, URL, &Headers::new(), b"other");
        assert!(matches!(r, Err(NetError::Mismatch("body"))));
        assert_eq!(connections_opened(), before);
        assert_eq!(g.attempts_left(), MAX_ATTEMPTS);
    }

    #[test]
    fn config_ignores_the_environment_and_refuses_redirects() {
        let c = client_config(Relax::none());
        assert!(c.proxy().is_none());
        assert_eq!(c.max_redirects(), 0);
        assert!(c.https_only());
        assert!(!c.http_status_as_error());
    }

    #[test]
    fn resolver_filter_keeps_only_public_addresses() {
        let sa = |s: &str| -> SocketAddr { s.parse().unwrap() };
        let all = [
            sa("127.0.0.1:443"),
            sa("10.0.0.7:443"),
            sa("8.8.8.8:443"),
            sa("[::1]:443"),
            sa("[::ffff:192.168.0.1]:443"),
            sa("[2606:4700::1111]:443"),
            sa("169.254.169.254:443"),
        ];
        let kept = vet_addrs(&all, false, Relax::none());
        assert_eq!(kept, vec![sa("8.8.8.8:443"), sa("[2606:4700::1111]:443")]);
        // A LAN-host grant also keeps private addresses, never loopback.
        let lan = vet_addrs(&all, true, Relax::none());
        assert_eq!(
            lan,
            vec![
                sa("10.0.0.7:443"),
                sa("8.8.8.8:443"),
                sa("[::ffff:192.168.0.1]:443"),
                sa("[2606:4700::1111]:443"),
            ]
        );
        assert!(vet_addrs(&[sa("127.0.0.1:1")], false, Relax::none()).is_empty());
    }

    #[test]
    fn hostname_resolving_to_loopback_is_blocked_before_connecting() {
        let _serial = serial();
        // `localhost` resolves to loopback only: the resolver refuses it and
        // the connector is never called.
        let mut g = mint("https://localhost/v1", b"b").unwrap();
        let before = connections_opened();
        let r = send(&mut g, "https://localhost/v1", &Headers::new(), b"b");
        assert!(matches!(r, Err(NetError::Blocked(_))), "{r:?}");
        assert_eq!(connections_opened(), before);
        // Blocked is final: not retryable.
        assert!(matches!(
            send(&mut g, "https://localhost/v1", &Headers::new(), b"b"),
            Err(NetError::Exhausted)
        ));
    }

    /// One scripted response per accepted connection; records the raw requests.
    struct Mock {
        port: u16,
        requests: Arc<Mutex<Vec<String>>>,
    }

    fn mock(responses: Vec<String>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        std::thread::spawn(move || {
            for resp in responses {
                let Ok((mut s, _)) = listener.accept() else {
                    return;
                };
                let mut buf = vec![0u8; 65536];
                let mut n = 0;
                // Read until the headers and declared body are in.
                loop {
                    let k = s.read(&mut buf[n..]).unwrap_or(0);
                    n += k;
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    if let Some(head_end) = text.find("\r\n\r\n") {
                        let len = text[..head_end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if n >= head_end + 4 + len {
                            break;
                        }
                    }
                    if k == 0 {
                        break;
                    }
                }
                log.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).to_string());
                let _ = s.write_all(resp.as_bytes());
            }
        });
        Mock { port, requests }
    }

    fn http(status: &str, extra: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn local_grant(port: u16, body: &[u8]) -> (CloudGrant, String) {
        let url = format!("http://127.0.0.1:{port}/v1/chat");
        let g = CloudGrant::mint_inner(Relax::loopback(), NetPolicy::Default, &url, body, &[OPEN])
            .unwrap();
        (g, url)
    }

    #[test]
    fn sends_bound_bytes_and_headers_and_reads_the_answer() {
        let _serial = serial();
        let m = mock(vec![http("200 OK", "", r#"{"ok":true}"#)]);
        let (mut g, url) = local_grant(m.port, b"{\"a\":1}");
        let headers = Headers::new()
            .plain("content-type", "application/json")
            .secret(
                "authorization",
                crate::Secret::new("Bearer test-key-not-real"),
            );
        let before = connections_opened();
        let r = send(&mut g, &url, &headers, b"{\"a\":1}").unwrap();
        assert_eq!((r.status, r.body.as_slice()), (200, &br#"{"ok":true}"#[..]));
        assert!(connections_opened() > before);
        let req = m.requests.lock().unwrap()[0].clone();
        assert!(req.starts_with("POST /v1/chat HTTP/1.1"), "{req}");
        assert!(
            req.to_ascii_lowercase()
                .contains("authorization: bearer test-key-not-real")
        );
        assert!(req.ends_with("{\"a\":1}"));
        // An answered grant is spent.
        assert!(matches!(
            send(&mut g, &url, &headers, b"{\"a\":1}"),
            Err(NetError::Exhausted)
        ));
    }

    #[test]
    fn redirects_are_refused_and_not_followed() {
        let _serial = serial();
        let m = mock(vec![http(
            "302 Found",
            "Location: http://127.0.0.1:1/elsewhere\r\n",
            "",
        )]);
        let (mut g, url) = local_grant(m.port, b"b");
        let r = send(&mut g, &url, &Headers::new(), b"b");
        assert!(matches!(r, Err(NetError::Redirect(302))), "{r:?}");
        assert_eq!(m.requests.lock().unwrap().len(), 1);
        assert!(matches!(
            send(&mut g, &url, &Headers::new(), b"b"),
            Err(NetError::Exhausted)
        ));
    }

    #[test]
    fn retries_only_429_and_5xx_with_identical_bytes_up_to_three_attempts() {
        let _serial = serial();
        let m = mock(vec![
            http("429 Too Many Requests", "", r#"{"e":"slow"}"#),
            http("503 Service Unavailable", "", "x"),
            http("500 Internal Server Error", "", "y"),
            http("200 OK", "", "never"),
        ]);
        let (mut g, url) = local_grant(m.port, b"same");
        for want in [429, 503, 500] {
            let r = send(&mut g, &url, &Headers::new(), b"same").unwrap();
            assert_eq!(r.status, want);
        }
        assert_eq!(g.attempts_left(), 0);
        assert!(matches!(
            send(&mut g, &url, &Headers::new(), b"same"),
            Err(NetError::Exhausted)
        ));
        let reqs = m.requests.lock().unwrap();
        assert_eq!(reqs.len(), 3);
        assert!(reqs.iter().all(|r| r.ends_with("same")));
    }

    #[test]
    fn a_4xx_answer_spends_the_grant() {
        let _serial = serial();
        let m = mock(vec![http("400 Bad Request", "", r#"{"error":"bad"}"#)]);
        let (mut g, url) = local_grant(m.port, b"b");
        let r = send(&mut g, &url, &Headers::new(), b"b").unwrap();
        assert_eq!(r.status, 400);
        assert!(matches!(
            send(&mut g, &url, &Headers::new(), b"b"),
            Err(NetError::Exhausted)
        ));
    }

    #[test]
    fn transport_failure_may_be_retried() {
        let _serial = serial();
        // Nothing listens on this port.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let (mut g, url) = local_grant(port, b"b");
        let r = send(&mut g, &url, &Headers::new(), b"b");
        assert!(matches!(&r, Err(NetError::Transport(_))), "{r:?}");
        assert_eq!(g.attempts_left(), MAX_ATTEMPTS - 1);
        assert!(!g.spent);
    }

    #[test]
    fn sha256_hex_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
