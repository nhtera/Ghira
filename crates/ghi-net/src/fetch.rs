// SPDX-License-Identifier: Apache-2.0
//! Model downloads: content-free GETs of pinned files from allowlisted hosts.
//!
//! Unlike the cloud path, a download may follow redirects (Hugging Face
//! answers `resolve/` URLs with a redirect to its CDN), so redirects are
//! followed here and only here, by [`fetch`], under these rules:
//! - https only; no credentials in the URL; port 443;
//! - every hop (the first URL and each `Location`) must be on the allowlist
//!   ([`MODEL_HOSTS`] plus hosts the caller names); at most [`MAX_REDIRECTS`];
//! - strict offline is refused before any socket is opened;
//! - the real transport uses the same vetted resolver as the cloud client
//!   (public addresses only, loopback never) and ignores the proxy environment;
//! - the bytes go to `<dest>.part` and are renamed into place only after the
//!   size and SHA-256 match, so a half or tampered file is never `dest`.
//!
//! A body that goes quiet for [`Control::idle_timeout`] is abandoned (the `.part`
//! stays, so the next attempt resumes), and a [`Control::cancel`] flag stops a
//! download between chunks, also keeping the `.part`.
//!
//! Requests carry no user content: just a URL naming a model file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use ureq::config::Config;
use ureq::http::Uri;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::unversioned::transport::time::Duration as UreqDuration;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, NextTimeout, Transport as UreqTransportTrait,
};

use crate::cloud::{CountingConnector, Relax, VettedResolver};
use crate::{Destination, NetError, NetPolicy, classify};

/// Hosts model files may come from: Hugging Face and its CDN (`*.hf.co`:
/// `cdn-lfs*`, `us.aws.cdn`, `cas-bridge.xethub`). Doc 06 §1.
pub const MODEL_HOSTS: &[&str] = &["huggingface.co", "hf.co"];
/// Redirect hops a download may follow.
pub const MAX_REDIRECTS: u8 = 5;
/// Copy and hash buffer.
const BUF: usize = 1024 * 1024;
/// Size of one body read handed from the reader thread.
const CHUNK: usize = 64 * 1024;
/// How often the consumer wakes to look at the cancel flag.
const TICK: Duration = Duration::from_millis(100);
/// Progress callbacks are at most this often (10 Hz), bar the first and last.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);
/// Longest the real transport waits on one socket read, a backstop under
/// [`Control::idle_timeout`]: a reader thread whose consumer gave up on a
/// stalled body ends with a timeout error within this long, and its socket
/// closes.
const READ_BACKSTOP: Duration = Duration::from_secs(60);
/// Default for [`Control::idle_timeout`].
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const TIMEOUT_CONNECT: Duration = Duration::from_secs(15);
const TIMEOUT_RESPONSE: Duration = Duration::from_secs(60);

/// What a transport hands back for one GET.
pub struct Response {
    pub status: u16,
    /// `Location` header, for 3xx.
    pub location: Option<String>,
    pub content_length: Option<u64>,
    /// `Content-Range` header, for 206.
    pub content_range: Option<String>,
    /// `Send`: the body is read on its own thread so a stall can be timed out.
    pub body: Box<dyn Read + Send>,
}

/// One HTTP GET, without following redirects. The real one is
/// [`UreqTransport`]; tests use a fake.
pub trait Transport {
    /// `range_from`: ask for `bytes=N-`.
    fn get(&self, url: &str, range_from: Option<u64>) -> Result<Response, NetError>;
}

/// Per-download controls: a stall limit and a cancel flag.
#[derive(Debug, Clone)]
pub struct Control {
    /// Abort when no body bytes arrive for this long (`Duration::ZERO`: never).
    pub idle_timeout: Duration,
    /// Set to `true` (from any thread) to stop; checked between chunks.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for Control {
    fn default() -> Control {
        Control {
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            cancel: None,
        }
    }
}

impl Control {
    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }
}

/// What the progress callback is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress<'a> {
    /// Bytes on disk so far (resumed prefix included) of `total`, from `source`
    /// (the host of the URL being fetched). At most 10 per second; the first
    /// (the resumed prefix) and the last (`done == total`) always come.
    Bytes {
        done: u64,
        total: u64,
        source: &'a str,
    },
    /// All bytes are in; the SHA-256 is being checked.
    Verifying,
}

#[derive(Debug, Clone, Default)]
pub struct FetchOpts {
    /// Lowercase hex SHA-256 the finished file must have.
    pub expected_sha256: String,
    pub expected_size: u64,
    /// Continue from an existing `<dest>.part`.
    pub resume: bool,
    /// Hosts allowed besides [`MODEL_HOSTS`] (mirrors named by the registry).
    pub extra_hosts: Vec<String>,
    pub control: Control,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchReport {
    /// Bytes received in this call (not counting a resumed prefix).
    pub downloaded: u64,
    /// Size of the `.part` kept from an earlier call and continued.
    pub resumed_from: u64,
    /// Redirect hops followed.
    pub redirects: u8,
    pub final_host: String,
}

#[derive(Debug)]
pub enum FetchError {
    /// Refused by policy before or during the download (strict offline, not
    /// https, host off the allowlist, redirect off the allowlist).
    Denied(String),
    /// The server answered something other than 200 or 206.
    Status(u16),
    TooManyRedirects,
    /// The caller cancelled. The `.part` is kept for a resume.
    Cancelled,
    /// No body bytes for this long. The `.part` is kept for a resume.
    IdleTimeout(Duration),
    Transport(String),
    Io(String),
    /// The stream ended early or overran. An early end keeps the `.part`.
    SizeMismatch {
        expected: u64,
        got: u64,
    },
    /// The finished bytes are not the pinned file. The `.part` is deleted.
    HashMismatch,
}

impl FetchError {
    /// Whether another source (a mirror) may be tried.
    pub fn is_policy_denial(&self) -> bool {
        matches!(self, FetchError::Denied(_))
    }
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Denied(d) => write!(f, "not allowed: {d}"),
            FetchError::Status(s) => write!(f, "the server answered {s}"),
            FetchError::TooManyRedirects => f.write_str("too many redirects"),
            FetchError::Cancelled => f.write_str("cancelled"),
            FetchError::IdleTimeout(d) => {
                write!(f, "no data for {} s; stopped", d.as_secs())
            }
            FetchError::Transport(d) => write!(f, "transport: {d}"),
            FetchError::Io(d) => write!(f, "file error: {d}"),
            FetchError::SizeMismatch { expected, got } => {
                write!(f, "size mismatch: expected {expected} bytes, got {got}")
            }
            FetchError::HashMismatch => f.write_str("SHA-256 mismatch; file discarded"),
        }
    }
}

impl std::error::Error for FetchError {}

fn io_err(e: io::Error) -> FetchError {
    FetchError::Io(e.to_string())
}

impl From<NetError> for FetchError {
    fn from(e: NetError) -> FetchError {
        match e {
            NetError::Denied(d) => FetchError::Denied(d),
            other => FetchError::Transport(other.to_string()),
        }
    }
}

/// `<dest>.part`, where the bytes land until verified.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// A URL a download hop may use: https, no credentials, port 443, and a host
/// on the allowlist (a LAN or IP-literal host is never one).
fn vet_url(url: &str, hosts: &[&str]) -> Result<String, FetchError> {
    let denied = |d: &str| FetchError::Denied(d.to_owned());
    let uri: Uri = url.parse().map_err(|_| denied("not a valid URL"))?;
    if uri.scheme_str() != Some("https") {
        return Err(denied("only https URLs are allowed"));
    }
    let authority = uri.authority().ok_or_else(|| denied("no host in URL"))?;
    if authority.as_str().contains('@') {
        return Err(denied("credentials in the URL are not allowed"));
    }
    if uri.port_u16().is_some_and(|p| p != 443) {
        return Err(denied("only port 443 is allowed"));
    }
    let host = uri.host().unwrap_or("").to_ascii_lowercase();
    if classify(&host, hosts) != Destination::AllowlistedHost {
        return Err(FetchError::Denied(format!("{host} is not an allowed host")));
    }
    Ok(host)
}

/// Resolves a `Location` against the URL it came from.
fn resolve_location(base: &str, location: &str) -> Result<String, FetchError> {
    if location.starts_with("https://") || location.starts_with("http://") {
        return Ok(location.to_owned());
    }
    let uri: Uri = base
        .parse()
        .map_err(|_| FetchError::Transport("bad base URL".into()))?;
    let authority = uri.authority().map(|a| a.as_str()).unwrap_or("");
    if location.starts_with('/') && !location.starts_with("//") {
        Ok(format!("https://{authority}{location}"))
    } else {
        Err(FetchError::Denied("unsupported redirect target".into()))
    }
}

/// Start offset of a `Content-Range: bytes START-END/TOTAL` header.
fn range_start(header: &str) -> Option<u64> {
    let rest = header.trim().strip_prefix("bytes")?.trim_start();
    rest.split('-').next()?.trim().parse().ok()
}

/// SHA-256 of a file, streaming.
fn hash_file(path: &Path) -> io::Result<Sha256> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; BUF];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(hasher);
        }
        hasher.update(&buf[..n]);
    }
}

/// GETs `url`, following allowlisted redirects; returns the final response and
/// how it got there.
fn open(
    url: &str,
    range_from: Option<u64>,
    hosts: &[&str],
    transport: &dyn Transport,
) -> Result<(Response, u8, String), FetchError> {
    let mut current = url.to_owned();
    let mut hops = 0u8;
    loop {
        let host = vet_url(&current, hosts)?;
        let resp = transport.get(&current, range_from)?;
        if !(300..400).contains(&resp.status) {
            return Ok((resp, hops, host));
        }
        hops += 1;
        if hops > MAX_REDIRECTS {
            return Err(FetchError::TooManyRedirects);
        }
        let location = resp
            .location
            .as_deref()
            .ok_or_else(|| FetchError::Transport("redirect without Location".into()))?;
        current = resolve_location(&current, location)?;
    }
}

/// Reads `body` on a thread, sending chunks (empty = end, `Err` = read error).
/// Dropping the receiver ends the thread at its next send; a read that never
/// returns would keep it blocked (the socket then dies with the connection).
fn spawn_reader(mut body: Box<dyn Read + Send>) -> mpsc::Receiver<Result<Vec<u8>, String>> {
    let (tx, rx) = mpsc::sync_channel(4);
    std::thread::spawn(move || {
        loop {
            let mut buf = vec![0u8; CHUNK];
            let msg = match body.read(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    Ok(buf)
                }
                Err(e) => Err(e.to_string()),
            };
            let end = !matches!(&msg, Ok(b) if !b.is_empty());
            if tx.send(msg).is_err() || end {
                return;
            }
        }
    });
    rx
}

/// The next chunk, or Cancelled / IdleTimeout.
fn next_chunk(
    rx: &mpsc::Receiver<Result<Vec<u8>, String>>,
    ctl: &Control,
) -> Result<Vec<u8>, FetchError> {
    let started = Instant::now();
    loop {
        if ctl.cancelled() {
            return Err(FetchError::Cancelled);
        }
        let mut wait = TICK;
        if !ctl.idle_timeout.is_zero() {
            let left = ctl.idle_timeout.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Err(FetchError::IdleTimeout(ctl.idle_timeout));
            }
            wait = wait.min(left);
        }
        match rx.recv_timeout(wait) {
            Ok(Ok(chunk)) => return Ok(chunk),
            Ok(Err(e)) => return Err(FetchError::Transport(e)),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(FetchError::Transport("body reader stopped".into()));
            }
        }
    }
}

/// Rate limit for the progress callback.
struct Ticker(Option<Instant>);

impl Ticker {
    fn due(&mut self, force: bool) -> bool {
        let now = Instant::now();
        if force
            || self
                .0
                .is_none_or(|t| now.duration_since(t) >= PROGRESS_EVERY)
        {
            self.0 = Some(now);
            true
        } else {
            false
        }
    }
}

/// Downloads `url` to `dest` (see the module docs). `progress` is told the
/// byte count (resumed prefix included) at most 10 times a second, then
/// [`Progress::Verifying`] once all bytes are in.
pub fn fetch(
    policy: NetPolicy,
    url: &str,
    dest: &Path,
    opts: &FetchOpts,
    progress: &mut dyn FnMut(Progress<'_>),
    transport: &dyn Transport,
) -> Result<FetchReport, FetchError> {
    if policy == NetPolicy::StrictOffline {
        return Err(FetchError::Denied("strict offline is on".into()));
    }
    let mut hosts: Vec<&str> = MODEL_HOSTS.to_vec();
    hosts.extend(opts.extra_hosts.iter().map(String::as_str));
    let source = vet_url(url, &hosts)?;
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).map_err(io_err)?;
    }
    let part = part_path(dest);
    if !opts.resume {
        let _ = fs::remove_file(&part);
    }
    let total = opts.expected_size;
    let mut restarted = false;
    loop {
        if opts.control.cancelled() {
            return Err(FetchError::Cancelled);
        }
        let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if have > total {
            let _ = fs::remove_file(&part);
            have = 0;
        }
        let mut hasher = Sha256::new();
        let mut report = FetchReport {
            downloaded: 0,
            resumed_from: have,
            redirects: 0,
            final_host: String::new(),
        };
        if have == total && have > 0 {
            // A complete .part (killed between the last byte and the rename).
            progress(Progress::Verifying);
            hasher = hash_file(&part).map_err(io_err)?;
            return finish(hasher, &part, dest, opts, report);
        }
        let (resp, redirects, host) = open(url, (have > 0).then_some(have), &hosts, transport)?;
        report.redirects = redirects;
        report.final_host = host;
        let start = match resp.status {
            200 => 0,
            206 if have > 0 => {
                let got = resp.content_range.as_deref().and_then(range_start);
                if got != Some(have) {
                    return Err(FetchError::Transport("unexpected Content-Range".into()));
                }
                have
            }
            416 if have > 0 && !restarted => {
                // The .part does not fit this file: drop it and start over once.
                restarted = true;
                let _ = fs::remove_file(&part);
                continue;
            }
            status => return Err(FetchError::Status(status)),
        };
        report.resumed_from = start;
        let mut file = if start == 0 {
            File::create(&part).map_err(io_err)?
        } else {
            hasher = hash_file(&part).map_err(io_err)?;
            OpenOptions::new()
                .append(true)
                .open(&part)
                .map_err(io_err)?
        };
        let mut done = start;
        let mut tick = Ticker(None);
        let bytes = |done| Progress::Bytes {
            done,
            total,
            source: &source,
        };
        progress(bytes(done));
        tick.due(true);
        let rx = spawn_reader(resp.body);
        loop {
            let chunk = next_chunk(&rx, &opts.control)?;
            if chunk.is_empty() {
                break;
            }
            done += chunk.len() as u64;
            if done > total {
                let _ = fs::remove_file(&part);
                return Err(FetchError::SizeMismatch {
                    expected: total,
                    got: done,
                });
            }
            hasher.update(&chunk);
            file.write_all(&chunk).map_err(io_err)?;
            report.downloaded += chunk.len() as u64;
            if tick.due(done == total) {
                progress(bytes(done));
            }
        }
        file.flush().map_err(io_err)?;
        drop(file);
        if done != total {
            // Truncated: the .part stays for a resume.
            return Err(FetchError::SizeMismatch {
                expected: total,
                got: done,
            });
        }
        progress(Progress::Verifying);
        return finish(hasher, &part, dest, opts, report);
    }
}

fn finish(
    hasher: Sha256,
    part: &Path,
    dest: &Path,
    opts: &FetchOpts,
    report: FetchReport,
) -> Result<FetchReport, FetchError> {
    if !hex(&hasher.finalize()).eq_ignore_ascii_case(&opts.expected_sha256) {
        let _ = fs::remove_file(part);
        return Err(FetchError::HashMismatch);
    }
    fs::rename(part, dest).map_err(io_err)?;
    Ok(report)
}

/// The real transport: rustls with the platform verifier, no proxy, a vetted
/// resolver (public addresses only), every attempt counted by
/// [`connections_opened`](crate::connections_opened).
#[derive(Debug, Default)]
pub struct UreqTransport;

impl Transport for UreqTransport {
    fn get(&self, url: &str, range_from: Option<u64>) -> Result<Response, NetError> {
        let blocked = Arc::new(AtomicBool::new(false));
        let resolver = VettedResolver {
            allow_lan: false,
            relax: Relax::none(),
            blocked: blocked.clone(),
        };
        let agent =
            ureq::Agent::with_parts(client_config(), ClampedConnector(READ_BACKSTOP), resolver);
        let mut req = agent.get(url);
        if let Some(from) = range_from {
            req = req.header("Range", format!("bytes={from}-"));
        }
        let resp = req.call().map_err(|e| {
            if blocked.load(Ordering::SeqCst) {
                NetError::Blocked(url.to_owned())
            } else {
                NetError::Transport(e.to_string())
            }
        })?;
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        Ok(Response {
            status: resp.status().as_u16(),
            location: header("location"),
            content_length: header("content-length").and_then(|v| v.parse().ok()),
            content_range: header("content-range"),
            body: Box::new(resp.into_body().into_reader()),
        })
    }
}

/// [`CountingConnector`]'s transport with every wait for input capped at the
/// given time: ureq has no per-read timeout, and its body timeout is one
/// deadline for the whole body.
#[derive(Debug)]
struct ClampedConnector(Duration);

impl Connector<()> for ClampedConnector {
    type Out = Box<dyn UreqTransportTrait>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(CountingConnector
            .connect(details, chained)?
            .map(|inner| Box::new(Clamped { inner, cap: self.0 }) as Box<dyn UreqTransportTrait>))
    }
}

#[derive(Debug)]
struct Clamped {
    inner: Box<dyn UreqTransportTrait>,
    cap: Duration,
}

impl UreqTransportTrait for Clamped {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.inner.transmit_output(amount, timeout)
    }

    fn maybe_await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.inner.maybe_await_input(clamp(timeout, self.cap))
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.inner.await_input(clamp(timeout, self.cap))
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

fn clamp(mut t: NextTimeout, cap: Duration) -> NextTimeout {
    let cap = UreqDuration::from(cap);
    if t.after > cap {
        t.after = cap;
    }
    t
}

/// No global timeout (a model is gigabytes), no redirects (the caller follows
/// them hop by hop), no proxy.
fn client_config() -> Config {
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    Config::builder()
        .proxy(None)
        .max_redirects(0)
        .https_only(true)
        .http_status_as_error(false)
        .timeout_global(None)
        .timeout_connect(Some(TIMEOUT_CONNECT))
        .timeout_recv_response(Some(TIMEOUT_RESPONSE))
        .tls_config(tls)
        .user_agent(concat!("ghira/", env!("CARGO_PKG_VERSION")))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io::Cursor;

    const DATA: &[u8] = b"0123456789abcdefghij";

    fn sha(data: &[u8]) -> String {
        hex(&Sha256::digest(data))
    }

    fn opts() -> FetchOpts {
        FetchOpts {
            expected_sha256: sha(DATA),
            expected_size: DATA.len() as u64,
            resume: true,
            extra_hosts: vec![],
            control: Control::default(),
        }
    }

    fn ok(status: u16, body: &[u8], content_range: Option<&str>) -> Response {
        Response {
            status,
            location: None,
            content_length: Some(body.len() as u64),
            content_range: content_range.map(str::to_owned),
            body: Box::new(Cursor::new(body.to_vec())),
        }
    }

    fn redirect(to: &str) -> Response {
        Response {
            status: 302,
            location: Some(to.to_owned()),
            content_length: Some(0),
            content_range: None,
            body: Box::new(io::empty()),
        }
    }

    /// Answers from a queue and records (url, range) of each call.
    struct Fake {
        replies: RefCell<VecDeque<Response>>,
        calls: RefCell<Vec<(String, Option<u64>)>>,
    }

    impl Fake {
        fn new(replies: Vec<Response>) -> Fake {
            Fake {
                replies: RefCell::new(replies.into()),
                calls: RefCell::new(vec![]),
            }
        }
    }

    impl Transport for Fake {
        fn get(&self, url: &str, range_from: Option<u64>) -> Result<Response, NetError> {
            self.calls.borrow_mut().push((url.to_owned(), range_from));
            self.replies
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| NetError::Transport("no reply queued".into()))
        }
    }

    const URL: &str = "https://huggingface.co/o/r/resolve/abc/model.gguf";

    fn run(
        url: &str,
        dir: &Path,
        o: &FetchOpts,
        t: &Fake,
        policy: NetPolicy,
    ) -> Result<FetchReport, FetchError> {
        fetch(policy, url, &dir.join("model.gguf"), o, &mut |_| {}, t)
    }

    #[test]
    fn follows_allowlisted_redirect_and_installs() {
        let dir = tempfile::tempdir().unwrap();
        let t = Fake::new(vec![
            redirect("https://cas-bridge.xethub.hf.co/x?sig=1"),
            ok(200, DATA, None),
        ]);
        let mut seen = vec![];
        let r = fetch(
            NetPolicy::Default,
            URL,
            &dir.path().join("model.gguf"),
            &opts(),
            &mut |p| {
                if let Progress::Bytes { done, total, .. } = p {
                    seen.push((done, total));
                }
            },
            &t,
        )
        .unwrap();
        assert_eq!(r.redirects, 1);
        assert_eq!(r.final_host, "cas-bridge.xethub.hf.co");
        assert_eq!(r.downloaded, 20);
        assert_eq!(seen.last(), Some(&(20, 20)));
        assert_eq!(fs::read(dir.path().join("model.gguf")).unwrap(), DATA);
        assert!(!part_path(&dir.path().join("model.gguf")).exists());
    }

    #[test]
    fn relative_redirect_stays_on_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let t = Fake::new(vec![redirect("/other/path"), ok(200, DATA, None)]);
        run(URL, dir.path(), &opts(), &t, NetPolicy::Default).unwrap();
        assert_eq!(t.calls.borrow()[1].0, "https://huggingface.co/other/path");
    }

    #[test]
    fn off_list_redirect_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for target in [
            "https://evil.example.com/m.gguf",
            "https://hf.co.evil.com/m.gguf",
            "http://cdn-lfs.hf.co/m.gguf",
            "https://127.0.0.1/m.gguf",
            "https://192.168.1.5/m.gguf",
        ] {
            let t = Fake::new(vec![redirect(target), ok(200, DATA, None)]);
            let e = run(URL, dir.path(), &opts(), &t, NetPolicy::Default).unwrap_err();
            assert!(matches!(e, FetchError::Denied(_)), "{target}: {e}");
            assert_eq!(t.calls.borrow().len(), 1, "{target}");
        }
        assert!(!dir.path().join("model.gguf").exists());
    }

    #[test]
    fn redirect_loop_stops() {
        let dir = tempfile::tempdir().unwrap();
        let t = Fake::new((0..10).map(|_| redirect(URL)).collect());
        let e = run(URL, dir.path(), &opts(), &t, NetPolicy::Default).unwrap_err();
        assert!(matches!(e, FetchError::TooManyRedirects));
        assert_eq!(t.calls.borrow().len(), MAX_REDIRECTS as usize + 1);
    }

    #[test]
    fn refuses_bad_urls_before_the_transport() {
        let dir = tempfile::tempdir().unwrap();
        for url in [
            "http://huggingface.co/x",
            "https://example.com/x",
            "https://user:pw@huggingface.co/x",
            "https://huggingface.co:8443/x",
            "https://127.0.0.1/x",
            "ftp://huggingface.co/x",
            "huggingface.co/x",
        ] {
            let t = Fake::new(vec![ok(200, DATA, None)]);
            let e = run(url, dir.path(), &opts(), &t, NetPolicy::Default).unwrap_err();
            assert!(matches!(e, FetchError::Denied(_)), "{url}");
            assert!(t.calls.borrow().is_empty(), "{url}");
        }
    }

    #[test]
    fn extra_hosts_extend_the_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let url = "https://models.example.org/m.gguf";
        let t = Fake::new(vec![ok(200, DATA, None)]);
        assert!(run(url, dir.path(), &opts(), &t, NetPolicy::Default).is_err());
        let mut o = opts();
        o.extra_hosts = vec!["models.example.org".into()];
        let t = Fake::new(vec![ok(200, DATA, None)]);
        assert!(run(url, dir.path(), &o, &t, NetPolicy::Default).is_ok());
    }

    #[test]
    fn strict_offline_is_refused_before_any_socket() {
        let dir = tempfile::tempdir().unwrap();
        let t = Fake::new(vec![ok(200, DATA, None)]);
        let e = run(URL, dir.path(), &opts(), &t, NetPolicy::StrictOffline).unwrap_err();
        assert!(matches!(e, FetchError::Denied(_)));
        assert!(t.calls.borrow().is_empty());
    }

    #[test]
    fn resumes_from_part_with_content_range() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        fs::write(part_path(&dest), &DATA[..8]).unwrap();
        let t = Fake::new(vec![ok(206, &DATA[8..], Some("bytes 8-19/20"))]);
        let r = run(URL, dir.path(), &opts(), &t, NetPolicy::Default).unwrap();
        assert_eq!(t.calls.borrow()[0].1, Some(8));
        assert_eq!((r.resumed_from, r.downloaded), (8, 12));
        assert_eq!(fs::read(&dest).unwrap(), DATA);
    }

    #[test]
    fn server_ignoring_range_restarts_from_zero() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        fs::write(part_path(&dest), b"garbage!").unwrap();
        let t = Fake::new(vec![ok(200, DATA, None)]);
        let r = run(URL, dir.path(), &opts(), &t, NetPolicy::Default).unwrap();
        assert_eq!((r.resumed_from, r.downloaded), (0, 20));
        assert_eq!(fs::read(&dest).unwrap(), DATA);
    }

    #[test]
    fn mismatched_content_range_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        fs::write(part_path(&dest), &DATA[..8]).unwrap();
        let t = Fake::new(vec![ok(206, &DATA[4..], Some("bytes 4-19/20"))]);
        assert!(run(URL, dir.path(), &opts(), &t, NetPolicy::Default).is_err());
        assert!(!dest.exists());
    }

    #[test]
    fn stale_part_after_416_restarts_once() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        fs::write(part_path(&dest), &DATA[..8]).unwrap();
        let t = Fake::new(vec![ok(416, b"", None), ok(200, DATA, None)]);
        run(URL, dir.path(), &opts(), &t, NetPolicy::Default).unwrap();
        assert_eq!(t.calls.borrow()[1].1, None);
        assert_eq!(fs::read(&dest).unwrap(), DATA);
    }

    #[test]
    fn complete_part_is_verified_without_a_request() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        fs::write(part_path(&dest), DATA).unwrap();
        let t = Fake::new(vec![]);
        run(URL, dir.path(), &opts(), &t, NetPolicy::Default).unwrap();
        assert!(t.calls.borrow().is_empty());
        assert_eq!(fs::read(&dest).unwrap(), DATA);
    }

    #[test]
    fn bad_hash_leaves_nothing_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        let mut bad = DATA.to_vec();
        bad[3] ^= 1;
        let t = Fake::new(vec![ok(200, &bad, None)]);
        let e = run(URL, dir.path(), &opts(), &t, NetPolicy::Default).unwrap_err();
        assert!(matches!(e, FetchError::HashMismatch));
        assert!(!dest.exists());
        assert!(!part_path(&dest).exists());
    }

    #[test]
    fn truncated_stream_keeps_part_and_overrun_drops_it() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        let t = Fake::new(vec![ok(200, &DATA[..10], None)]);
        let e = run(URL, dir.path(), &opts(), &t, NetPolicy::Default).unwrap_err();
        assert!(matches!(
            e,
            FetchError::SizeMismatch {
                expected: 20,
                got: 10
            }
        ));
        assert_eq!(fs::metadata(part_path(&dest)).unwrap().len(), 10);
        let mut long = DATA.to_vec();
        long.push(b'!');
        let t = Fake::new(vec![ok(200, &long, None)]);
        let mut o = opts();
        o.resume = false;
        let e = run(URL, dir.path(), &o, &t, NetPolicy::Default).unwrap_err();
        assert!(matches!(e, FetchError::SizeMismatch { .. }));
        assert!(!part_path(&dest).exists());
    }

    #[test]
    fn http_errors_surface_as_status() {
        let dir = tempfile::tempdir().unwrap();
        let t = Fake::new(vec![ok(404, b"", None)]);
        let e = run(URL, dir.path(), &opts(), &t, NetPolicy::Default).unwrap_err();
        assert!(matches!(e, FetchError::Status(404)));
    }

    /// Yields `DATA[..n]` in `step`-byte reads, then blocks (a stalled socket)
    /// or ends.
    struct Trickle {
        data: Vec<u8>,
        pos: usize,
        step: usize,
        stall_at: Option<usize>,
        delay: Duration,
    }

    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.stall_at == Some(self.pos) {
                std::thread::sleep(Duration::from_secs(5));
            }
            std::thread::sleep(self.delay);
            let end = (self.pos + self.step).min(self.data.len());
            let n = (end - self.pos).min(buf.len());
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    fn trickle(data: &[u8], step: usize, stall_at: Option<usize>) -> Response {
        trickle_slow(data, step, stall_at, Duration::ZERO)
    }

    fn trickle_slow(
        data: &[u8],
        step: usize,
        stall_at: Option<usize>,
        delay: Duration,
    ) -> Response {
        Response {
            status: 200,
            location: None,
            content_length: Some(data.len() as u64),
            content_range: None,
            body: Box::new(Trickle {
                data: data.to_vec(),
                pos: 0,
                step,
                stall_at,
                delay,
            }),
        }
    }

    #[test]
    fn stalled_body_times_out_then_resumes_from_the_part() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        let mut o = opts();
        o.control.idle_timeout = Duration::from_millis(300);
        // 8 bytes arrive, then the socket goes quiet.
        let t = Fake::new(vec![trickle(DATA, 4, Some(8))]);
        let started = Instant::now();
        let e = run(URL, dir.path(), &o, &t, NetPolicy::Default).unwrap_err();
        assert!(matches!(e, FetchError::IdleTimeout(_)), "{e}");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(fs::metadata(part_path(&dest)).unwrap().len(), 8);
        // The next attempt continues from byte 8 and completes.
        let t = Fake::new(vec![ok(206, &DATA[8..], Some("bytes 8-19/20"))]);
        let r = run(URL, dir.path(), &o, &t, NetPolicy::Default).unwrap();
        assert_eq!(t.calls.borrow()[0].1, Some(8));
        assert_eq!((r.resumed_from, r.downloaded), (8, 12));
        assert_eq!(fs::read(&dest).unwrap(), DATA);
    }

    #[test]
    fn cancel_mid_body_keeps_the_part_and_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        let cancel = Arc::new(AtomicBool::new(false));
        let mut o = opts();
        o.control.cancel = Some(cancel.clone());
        let t = Fake::new(vec![trickle_slow(DATA, 5, None, Duration::from_millis(60))]);
        let flag = cancel.clone();
        let e = fetch(
            NetPolicy::Default,
            URL,
            &dest,
            &o,
            &mut |p| {
                if matches!(p, Progress::Bytes { done, .. } if done > 0) {
                    flag.store(true, Ordering::Relaxed);
                }
            },
            &t,
        )
        .unwrap_err();
        assert!(matches!(e, FetchError::Cancelled), "{e}");
        let kept = fs::metadata(part_path(&dest)).unwrap().len();
        assert!(kept > 0 && kept < 20, "{kept}");
        assert!(!dest.exists());
        // Cancelled before it starts: no request at all.
        let t0 = Fake::new(vec![]);
        let e = run(URL, dir.path(), &o, &t0, NetPolicy::Default).unwrap_err();
        assert!(matches!(e, FetchError::Cancelled));
        assert!(t0.calls.borrow().is_empty());
        // Resume.
        cancel.store(false, Ordering::Relaxed);
        let range = format!("bytes {kept}-19/20");
        let t = Fake::new(vec![ok(206, &DATA[kept as usize..], Some(&range))]);
        run(URL, dir.path(), &o, &t, NetPolicy::Default).unwrap();
        assert_eq!(t.calls.borrow()[0].1, Some(kept));
        assert_eq!(fs::read(&dest).unwrap(), DATA);
    }

    #[test]
    fn cancel_interrupts_a_stalled_wait() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut o = opts();
        o.control.cancel = Some(cancel.clone());
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            flag.store(true, Ordering::Relaxed);
        });
        let t = Fake::new(vec![trickle(DATA, 4, Some(4))]);
        let started = Instant::now();
        let e = run(URL, dir.path(), &o, &t, NetPolicy::Default).unwrap_err();
        assert!(matches!(e, FetchError::Cancelled), "{e}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn progress_is_monotonic_rate_limited_and_ends_verifying() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("model.gguf");
        let t = Fake::new(vec![trickle(DATA, 1, None)]);
        let mut events = vec![];
        fetch(
            NetPolicy::Default,
            URL,
            &dest,
            &opts(),
            &mut |p| events.push(format!("{p:?}")),
            &t,
        )
        .unwrap();
        // 20 one-byte reads arrive in well under 100 ms: first, last, Verifying.
        let bytes: Vec<u64> = events
            .iter()
            .filter_map(|e| {
                let rest = e.strip_prefix("Bytes { done: ")?;
                rest.split(',').next()?.parse().ok()
            })
            .collect();
        assert!(bytes.windows(2).all(|w| w[0] <= w[1]), "{bytes:?}");
        assert_eq!(bytes.first(), Some(&0));
        assert_eq!(bytes.last(), Some(&20));
        assert!(bytes.len() <= 4, "{bytes:?}");
        assert_eq!(events.last().unwrap(), "Verifying");
        assert!(events.iter().all(|e| !e.contains("source: \"\"")));
        assert!(events[0].contains("huggingface.co"));
    }

    #[test]
    fn strict_offline_reports_nothing_and_opens_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let t = Fake::new(vec![ok(200, DATA, None)]);
        let mut n = 0;
        let e = fetch(
            NetPolicy::StrictOffline,
            URL,
            &dir.path().join("model.gguf"),
            &opts(),
            &mut |_| n += 1,
            &t,
        )
        .unwrap_err();
        assert!(matches!(e, FetchError::Denied(_)));
        assert_eq!(n, 0);
        assert!(t.calls.borrow().is_empty());
    }

    #[test]
    fn read_waits_are_capped_at_the_backstop() {
        use ureq::Timeout;
        let next = |after| NextTimeout {
            after,
            reason: Timeout::RecvBody,
        };
        let cap = Duration::from_secs(60);
        for long in [UreqDuration::NotHappening, UreqDuration::from_secs(3600)] {
            assert_eq!(clamp(next(long), cap).after, UreqDuration::from_secs(60));
        }
        let short = next(UreqDuration::from_secs(5));
        assert_eq!(clamp(short, cap), short);
    }

    #[test]
    fn parses_content_range() {
        assert_eq!(range_start("bytes 100-199/200"), Some(100));
        assert_eq!(range_start("bytes */200"), None);
        assert_eq!(range_start("nope"), None);
    }
}
