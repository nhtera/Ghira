// SPDX-License-Identifier: Apache-2.0
//! `ghi sync serve|pair|run|status|export|import`: LAN sync between two data directories
//! (phase 15, doc 07). `serve` is the hub (listener on the machine's private
//! addresses, mDNS with the `mdns` feature), `pair` and `run` are a spoke.
//! `export` and `import` are the fallback for devices that can't reach each
//! other: a passphrase-sealed file (doc 07 §10), the passphrase from a TTY
//! prompt, `GHI_EXPORT_PASSPHRASE`, or the first line of stdin, never an
//! argument and never printed.
//!
//! The pairing code carries a one-time secret, so `serve` prints it only with
//! `--print-qr` (stdout, one line); otherwise it writes an SVG next to the
//! data directory (mode 0600, removed on exit) and prints just its path.
//! Nothing here prints keys, PSKs or pairing codes in logs or errors.

use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use clap::{Args, Subcommand};
use ghi_net::lan;
use ghi_store::store::Store;
use ghi_sync::SyncStore;
use ghi_sync::clock::SystemClock;
use ghi_sync::identity::Identity;
use ghi_sync::qr;
use ghi_sync::service::{HubNode, Served, pair_via, paired_hub, run_session};
use serde_json::json;

use crate::cmd::record::{STOP, install_signal_handlers};
use crate::contract::{ErrorCode, ErrorDoc};
use crate::keystore::{open_store, store_error, sync_secrets};

/// How often the listener follows the machine's addresses.
const REBIND_EVERY: Duration = Duration::from_secs(10);
/// How long `run` waits for mDNS to find a hub.
#[cfg_attr(not(feature = "mdns"), allow(dead_code))]
const DISCOVER_FOR: Duration = Duration::from_secs(5);

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Hub: listen on the private LAN addresses, advertise over mDNS and open
    /// a pairing window; serves sessions until Ctrl-C (`ghi.sync-serve/1`).
    Serve(ServeArgs),
    /// Spoke: pair with a hub from its pairing code (`ghi.sync-pair/1`).
    Pair(PairArgs),
    /// Spoke: run one session with the paired hub (`ghi.sync-run/1`).
    Run(RunArgs),
    /// Paired devices and sync cursors; no secrets (`ghi.sync-status/1`).
    Status(StatusArgs),
    /// Writes a passphrase-sealed file with meetings, their audio and keys,
    /// for a device that can't sync over the network (`ghi.sync-export/1`).
    Export(ExportArgs),
    /// Merges a sealed export file into this store; the same merge as a sync,
    /// no pairing needed (`ghi.sync-import/1`).
    Import(ImportArgs),
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// The data directory (the hub's store).
    #[arg(long)]
    pub dir: PathBuf,
    /// Listen on this private address only (default: every private address).
    #[arg(long)]
    pub bind: Option<IpAddr>,
    /// Listen on this port (default: any free one).
    #[arg(long, default_value_t = 0)]
    pub port: u16,
    /// Name the phone shows for this computer.
    #[arg(long, default_value = "Ghira CLI")]
    pub name: String,
    /// Print the pairing code to stdout (it carries a secret; keep it out of
    /// logs). Without it an SVG is written next to the data directory.
    #[arg(long)]
    pub print_qr: bool,
    /// Do not open a pairing window: only paired devices can connect.
    #[arg(long)]
    pub no_pair: bool,
}

#[derive(Debug, Args)]
pub struct PairArgs {
    /// The data directory (the spoke's store).
    #[arg(long)]
    pub dir: PathBuf,
    /// The pairing code text.
    #[arg(long, conflicts_with = "qr_file")]
    pub qr: Option<String>,
    /// A file holding the pairing code (keeps it out of the process list).
    #[arg(long)]
    pub qr_file: Option<PathBuf>,
    /// Name the hub shows for this device.
    #[arg(long, default_value = "Ghira CLI")]
    pub name: String,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// The data directory (the spoke's store).
    #[arg(long)]
    pub dir: PathBuf,
    /// Connect to this address first (default: the last one that worked, then
    /// mDNS).
    #[arg(long)]
    pub peer: Option<SocketAddr>,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    #[arg(long)]
    pub dir: PathBuf,
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// The data directory to export from.
    #[arg(long)]
    pub dir: PathBuf,
    /// The file to write.
    #[arg(long)]
    pub out: PathBuf,
    /// A meeting to export (repeatable; default: every finished meeting).
    #[arg(long = "meeting")]
    pub meetings: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ImportArgs {
    /// The data directory to import into.
    #[arg(long)]
    pub dir: PathBuf,
    /// The export file.
    pub file: PathBuf,
}

pub fn run(action: &Action) -> Result<(), ErrorDoc> {
    match action {
        Action::Serve(a) => serve(a),
        Action::Pair(a) => pair(a),
        Action::Run(a) => run_once(a),
        Action::Status(a) => status(&a.dir),
        Action::Export(a) => export(a),
        Action::Import(a) => import(a),
    }
}

fn bad(msg: impl Into<String>) -> ErrorDoc {
    ErrorDoc::new(ErrorCode::BadInput, msg)
}

fn internal(what: &str, e: impl std::fmt::Display) -> ErrorDoc {
    ErrorDoc::new(ErrorCode::Internal, format!("{what}: {e}"))
}

fn sync_error(e: ghi_sync::SyncError) -> ErrorDoc {
    internal("sync", e)
}

/// The store and this device's sync identity (made on first use, under the
/// gid in the store's settings).
fn open(dir: &Path) -> Result<(Arc<Store>, Identity), ErrorDoc> {
    let store = Arc::new(open_store(dir)?);
    let secrets = sync_secrets(dir)?;
    let gid = store.sync_device_gid().map_err(store_error)?;
    let identity = Identity::load_or_create(secrets.as_ref(), &gid).map_err(sync_error)?;
    if identity.device_gid != gid {
        return Err(internal(
            "sync",
            "the stored identity belongs to another store",
        ));
    }
    Ok((store, identity))
}

fn as_sync(store: &Arc<Store>) -> Arc<dyn SyncStore> {
    store.clone()
}

fn qr_text(a: &PairArgs) -> Result<String, ErrorDoc> {
    match (&a.qr, &a.qr_file) {
        (Some(t), _) => Ok(t.trim().to_string()),
        (None, Some(p)) => std::fs::read_to_string(p)
            .map(|t| t.trim().to_string())
            .map_err(|e| bad(format!("{}: {e}", p.display()))),
        (None, None) => Err(bad("give --qr or --qr-file")),
    }
}

/// Writes `bytes` to a new file only the owner can read.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(bytes)
}

/// `<dir>.sync-qr.svg`, a sibling of the data directory.
fn qr_svg_path(dir: &Path) -> Result<PathBuf, ErrorDoc> {
    let abs = std::path::absolute(dir).map_err(|e| bad(format!("{}: {e}", dir.display())))?;
    let mut name = abs
        .file_name()
        .ok_or_else(|| bad("name the data directory itself"))?
        .to_owned();
    name.push(".sync-qr.svg");
    Ok(abs.with_file_name(name))
}

/// Removes the pairing code file when `serve` ends, however it ends.
struct RemoveOnDrop(Option<PathBuf>);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn serve(a: &ServeArgs) -> Result<(), ErrorDoc> {
    let (store, identity) = open(&a.dir)?;
    let addrs: Vec<IpAddr> = match a.bind {
        Some(ip) => vec![ip],
        None => lan::lan_addrs(),
    };
    if addrs.is_empty() {
        return Err(bad("no private LAN address to listen on (see --bind)"));
    }
    let mut listener =
        lan::Listener::bind(&addrs, a.port).map_err(|e| bad(format!("cannot listen: {e}")))?;
    let port = listener.port();
    let hub_gid = identity.device_gid.clone();
    let node = Arc::new(HubNode::new(
        as_sync(&store),
        identity,
        Arc::new(SystemClock),
        &a.name,
        port,
    ));

    let mut _svg = RemoveOnDrop(None);
    if !a.no_pair {
        let text = node
            .open_pairing_code(&listener.local_addrs())
            .map_err(sync_error)?;
        if a.print_qr {
            let mut out = std::io::stdout().lock();
            writeln!(out, "{text}")
                .and_then(|()| out.flush())
                .map_err(|e| internal("writing stdout", e))?;
        } else {
            let svg = qr::svg(&text).map_err(sync_error)?;
            let path = qr_svg_path(&a.dir)?;
            write_private(&path, svg.as_bytes())
                .map_err(|e| internal(&path.display().to_string(), e))?;
            crate::warn(&format!(
                "pairing code written to {} (valid {} s, removed on exit)",
                path.display(),
                ghi_sync::pair::QR_TTL.as_secs()
            ));
            _svg = RemoveOnDrop(Some(path));
        }
    }
    crate::warn(&format!(
        "listening on {} ({} s pairing window: {})",
        listener
            .local_addrs()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        ghi_sync::pair::QR_TTL.as_secs(),
        if a.no_pair { "off" } else { "open" }
    ));
    #[cfg(feature = "mdns")]
    let _advert = match lan::MdnsAdvertiser::start(&addrs, port) {
        Ok(ad) => {
            crate::warn("advertising over mDNS");
            Some(ad)
        }
        Err(e) => {
            crate::warn(&format!("mDNS advertising failed: {e}"));
            None
        }
    };

    install_signal_handlers();
    let limits = listener.limits();
    let (sessions, pairings, failures) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
    );
    let mut last_rebind = Instant::now();
    while !STOP.load(Ordering::Relaxed) {
        if a.bind.is_none() && last_rebind.elapsed() >= REBIND_EVERY {
            last_rebind = Instant::now();
            if let Err(e) = listener.rebind(&lan::lan_addrs()) {
                crate::warn(&format!("listener: {e}"));
            }
        }
        let (stream, peer) = match listener.accept_timeout(Duration::from_millis(250)) {
            Ok(Some(got)) => got,
            Ok(None) => continue,
            Err(e) => return Err(internal("listener", e)),
        };
        let (node, limits) = (Arc::clone(&node), Arc::clone(&limits));
        let (sessions, pairings, failures) = (
            Arc::clone(&sessions),
            Arc::clone(&pairings),
            Arc::clone(&failures),
        );
        std::thread::spawn(
            move || match node.serve(stream, Some(peer.ip()), Some(&limits)) {
                Ok(Served::Paired(p)) => {
                    pairings.fetch_add(1, Ordering::Relaxed);
                    crate::warn(&format!("paired with {} ({})", p.name, p.device_gid));
                }
                Ok(Served::Session(r)) => {
                    sessions.fetch_add(1, Ordering::Relaxed);
                    crate::warn(&format!(
                        "session: {} rows in, {} rows out, {} deletes in, {} deletes out, {} tracks",
                        r.rows_pushed,
                        r.rows_pulled,
                        r.tombs_pushed,
                        r.tombs_pulled,
                        r.tracks_received.len()
                    ));
                }
                Err(e) => {
                    failures.fetch_add(1, Ordering::Relaxed);
                    crate::warn(&format!("connection ended: {e}"));
                }
            },
        );
    }
    crate::emit(&json!({
        "schema": "ghi.sync-serve/1",
        "device_gid": hub_gid,
        "port": port,
        "sessions": sessions.load(Ordering::Relaxed),
        "pairings": pairings.load(Ordering::Relaxed),
        "failures": failures.load(Ordering::Relaxed),
    }))
}

fn pair(a: &PairArgs) -> Result<(), ErrorDoc> {
    let text = qr_text(a)?;
    let payload = qr::parse(&text).map_err(|e| bad(e.to_string()))?;
    let (store, identity) = open(&a.dir)?;
    let platform = std::env::consts::OS;
    let (paired, addr) =
        pair_via(store.as_ref(), &identity, &payload, &a.name, platform).map_err(sync_error)?;
    crate::emit(&json!({
        "schema": "ghi.sync-pair/1",
        "hub": {"gid": paired.device_gid, "name": paired.name},
        "addr": addr.to_string(),
    }))
}

/// Candidates from mDNS (empty without the `mdns` feature).
#[cfg(feature = "mdns")]
fn discover() -> Vec<SocketAddr> {
    use ghi_net::lan::Discovery;
    let found = match lan::MdnsDiscovery::start() {
        Ok(d) => d,
        Err(e) => {
            crate::warn(&format!("mDNS browse failed: {e}"));
            return Vec::new();
        }
    };
    let until = Instant::now() + DISCOVER_FOR;
    while Instant::now() < until {
        let c = found.candidates();
        if !c.is_empty() {
            return c;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Vec::new()
}

#[cfg(not(feature = "mdns"))]
fn discover() -> Vec<SocketAddr> {
    Vec::new()
}

fn run_once(a: &RunArgs) -> Result<(), ErrorDoc> {
    let (store, identity) = open(&a.dir)?;
    let hub = paired_hub(store.as_ref()).map_err(sync_error)?;
    let mut first: Vec<SocketAddr> = Vec::new();
    first.extend(a.peer);
    first.extend(
        hub.last_addr
            .as_deref()
            .and_then(|s| s.parse::<SocketAddr>().ok()),
    );
    let clock = Arc::new(SystemClock);
    let tried = if first.is_empty() {
        Err(ghi_sync::SyncError::Wire("no address to try".into()))
    } else {
        run_session(as_sync(&store), clock.clone(), &identity, &first)
    };
    let (report, addr) = match tried {
        Ok(done) => done,
        // The remembered address is only a hint: ask the network.
        Err(first_error) if a.peer.is_none() => {
            let found = discover();
            if found.is_empty() {
                return Err(sync_error(first_error));
            }
            run_session(as_sync(&store), clock, &identity, &found).map_err(sync_error)?
        }
        Err(e) => return Err(sync_error(e)),
    };
    crate::emit(&json!({
        "schema": "ghi.sync-run/1",
        "hub": {"gid": hub.gid, "name": hub.name},
        "addr": addr.to_string(),
        "rows_pushed": report.rows_pushed,
        "rows_pulled": report.rows_pulled,
        "tombs_pushed": report.tombs_pushed,
        "tombs_pulled": report.tombs_pulled,
        "tracks_sent": report.tracks_sent,
        "needs_confirm": report.needs_confirm,
    }))
}

fn status(dir: &Path) -> Result<(), ErrorDoc> {
    let store = open_store(dir)?;
    let devices = store.devices().map_err(store_error)?;
    let list: Vec<_> = devices
        .iter()
        .map(|d| {
            json!({
                "gid": d.gid,
                "name": d.name,
                "platform": d.platform,
                "role": d.role.as_str(),
                "state": format!("{:?}", d.state),
                "paired_at": d.paired_at,
                "last_seen": d.last_seen,
                "last_addr": d.last_addr,
                "push_seq": d.push_seq,
                "pull_seq": d.pull_seq,
                "pull_feed_id": d.pull_feed_id,
            })
        })
        .collect();
    crate::emit(&json!({
        "schema": "ghi.sync-status/1",
        "device_gid": store.sync_device_gid().map_err(store_error)?,
        "feed_id": store.feed_id().map_err(store_error)?,
        "devices": list,
    }))
}

/// The export passphrase: `GHI_EXPORT_PASSPHRASE`, else a prompt on the
/// terminal (no echo; asked twice when `confirm`), else the first line of stdin.
fn passphrase(confirm: bool) -> Result<zeroize::Zeroizing<String>, ErrorDoc> {
    use std::io::IsTerminal;
    if let Some(p) = std::env::var_os("GHI_EXPORT_PASSPHRASE").filter(|p| !p.is_empty()) {
        let p = p
            .into_string()
            .map_err(|_| bad("GHI_EXPORT_PASSPHRASE is not valid text"))?;
        return Ok(zeroize::Zeroizing::new(p));
    }
    if !std::io::stdin().is_terminal() {
        return crate::cmd::store::read_secret_line("passphrase");
    }
    let first = prompt_secret("Passphrase: ")?;
    if confirm && *first != *prompt_secret("Again: ")? {
        return Err(bad("the passphrases differ"));
    }
    Ok(first)
}

/// One line from the terminal with echo off, prompt on stderr.
fn prompt_secret(prompt: &str) -> Result<zeroize::Zeroizing<String>, ErrorDoc> {
    use std::io::BufRead;
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let echo = EchoOff::new();
    let mut line = zeroize::Zeroizing::new(String::new());
    let read = std::io::stdin().lock().read_line(&mut line);
    drop(echo);
    eprintln!();
    read.map_err(|e| bad(format!("reading the passphrase: {e}")))?;
    let len = line.trim_end_matches(['\r', '\n']).len();
    line.truncate(len);
    if line.is_empty() {
        return Err(bad("empty passphrase"));
    }
    Ok(line)
}

/// Terminal echo off while alive (a no-op where the terminal can't be set).
struct EchoOff {
    #[cfg(unix)]
    saved: Option<libc::termios>,
}

impl EchoOff {
    #[cfg(unix)]
    fn new() -> Self {
        // SAFETY: a zeroed termios is a valid out-parameter for tcgetattr,
        // and both calls only touch the stdin terminal.
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut t) != 0 {
                return EchoOff { saved: None };
            }
            let saved = t;
            t.c_lflag &= !libc::ECHO;
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &t) != 0 {
                return EchoOff { saved: None };
            }
            EchoOff { saved: Some(saved) }
        }
    }
    #[cfg(not(unix))]
    fn new() -> Self {
        EchoOff {}
    }
}

#[cfg(unix)]
impl Drop for EchoOff {
    fn drop(&mut self) {
        if let Some(t) = &self.saved {
            // SAFETY: restores the attributes read in `new`.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, t);
            }
        }
    }
}

fn export(a: &ExportArgs) -> Result<(), ErrorDoc> {
    let pass = passphrase(true)?;
    let store = open_store(&a.dir)?;
    let only = (!a.meetings.is_empty()).then_some(a.meetings.as_slice());
    let r = ghi_sync::export::export_for_device(&store, only, &pass, &a.out).map_err(sync_error)?;
    crate::emit(&json!({
        "schema": "ghi.sync-export/1",
        "file": a.out.display().to_string(),
        "meetings": r.meetings,
        "tombstones": r.tombstones,
        "tracks": r.tracks,
        "audio_bytes": r.audio_bytes,
    }))
}

fn import(a: &ImportArgs) -> Result<(), ErrorDoc> {
    let pass = passphrase(false)?;
    let store = open_store(&a.dir)?;
    let r = ghi_sync::export::import_from_device(&store, &a.file, &pass).map_err(|e| match e {
        // A wrong passphrase or a damaged file is the user's to fix.
        ghi_sync::SyncError::Store(e) => store_error(e),
        e => sync_error(e),
    })?;
    crate::emit(&json!({
        "schema": "ghi.sync-import/1",
        "meetings": r.meetings,
        "refused": r.refused,
        "tombstones": r.tombstones,
        "tracks": r.tracks,
        "audio_bytes": r.audio_bytes,
    }))
}
