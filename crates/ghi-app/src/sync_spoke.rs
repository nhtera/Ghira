// SPDX-License-Identifier: Apache-2.0
//! The phone's side of the sync service (phase 15, slice 15-J2; doc 07 §3.4,
//! §4.1, §8, §9). A child of `sync_service`: it shares the service's state and
//! helpers, and the desktop never starts any of it.
//!
//! **Pairing.** `sync_pair_scan_start` runs the platform's camera scan
//! ([`SpokeLink::scan`]), parses the code and pairs with the hub in it
//! (`pair_over`). Failures come back from the command as `invalid`, `expired`,
//! `cameraOff`, `localNetwork` or `upgradeRequired`; an unreachable or
//! refusing hub is a [`SyncEvent::Error`]. The code is a secret: it is parsed
//! and dropped, never logged.
//!
//! **The session loop** (one thread) keeps one session to the hub while the
//! app is in the foreground, unlocked, sync is on and a hub is paired: a full
//! pass at start, every 30 s (5 s while a lease is open, for the percent), on a
//! local change and when a `Pong` says the hub has news. Addresses are tried
//! in this order: the last good one, those from the pairing code, what the
//! platform's Bonjour browse found. Going to the background ends the session
//! with `Bye` inside a background task; the lock ends it too. A refused
//! handshake never drops the pin (anything on the LAN can close one): the UI
//! says it can't reach the computer, and a hub that unpairs the phone delivers
//! `Control::Unpair` inside a session.
//!
//! **Processing on the desktop.** A meeting recorded or imported with the
//! `Desktop` target is listed in [`DESKTOP_PENDING_KEY`] and gets no local job.
//! Once its rows, key and audio are acked, the loop offers a lease (the grantor
//! side of doc 07 §8); it is sent in the next pass. A revoke ("Process on this
//! phone now") and a lease whose hub stayed away for the configured hours both
//! end in a local final pass at `epoch + 1` (`self_take`), the second only on
//! a phone that can run one.

use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;

pub use ghi_sync::lease::GrantorState;
use ghi_sync::lease::{Grantor, GrantorAction, TakeBack};
use ghi_sync::service::{initiate_hub, pair_over, paired_hub};
use ghi_sync::session::spoke::SpokeSession;
use ghi_sync::transport::NoiseTransport;
use ghi_sync::wire::Control;

use super::*;

// The store setting listing the meetings that wait to be handed to the
// desktop (device-local; never synced). Crash recovery skips them.
pub use ghi_core::recover::DESKTOP_PENDING_KEY;

/// How often an idle session does a full pass.
const IDLE_PASS: Duration = Duration::from_secs(30);
/// How often it does while a lease is open (the chip's percent).
const LEASE_PASS: Duration = Duration::from_secs(5);
/// A local change starts a pass this long after the last one.
const CHANGE_DEBOUNCE: Duration = Duration::from_secs(3);
/// `Ping` while idle.
const PING_EVERY: Duration = Duration::from_secs(15);
/// The loop looks again this often (and at once when poked).
const LOOP_TICK: Duration = Duration::from_secs(1);
/// How often leases are checked for a hub that stayed away.
const POLL_EVERY: Duration = Duration::from_secs(5);
/// How long a command waits for the loop to let go of its connection.
const HOLD_WAIT: Duration = Duration::from_secs(5);
/// How long the camera scan waits for a code.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(120);

pub const ERR_NOT_SPOKE: &str = "sync_not_available";
pub const ERR_ALREADY_PAIRED: &str = "already_paired";
pub const ERR_NOT_CAPABLE: &str = "deviceNotCapable";
pub const ERR_NO_LEASE: &str = "no_open_lease";
/// Pairing failures the command returns (the UI words them).
pub const SCAN_INVALID: &str = "invalid";
pub const SCAN_EXPIRED: &str = "expired";
pub const SCAN_CAMERA_OFF: &str = "cameraOff";
pub const SCAN_LOCAL_NETWORK: &str = "localNetwork";
pub const SCAN_UPGRADE: &str = "upgradeRequired";

/// Why a camera scan gave no code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanError {
    /// The camera permission is off.
    Denied,
    /// No camera (or no scanner) here.
    Unavailable,
    /// The user closed the scanner, or [`SyncService::pair_scan_stop`] did.
    Cancelled,
    Timeout,
}

/// What the phone's platform gives the spoke: the camera, Bonjour, background
/// time and the device's tier. The sockets are the default [`SpokeLink::connect`]
/// (`ghi-net` is the one place that opens them); tests replace it with a pipe.
pub trait SpokeLink: Send + Sync {
    /// Opens a stream to `addr` (a LAN address, within the connect timeout).
    fn connect(&self, addr: SocketAddr) -> io::Result<Box<dyn ByteStream>> {
        match lan::connect(addr, lan::CONNECT_TIMEOUT) {
            Ok(s) => Ok(Box::new(s)),
            Err(lan::LanError::Io(e)) => Err(e),
            Err(lan::LanError::Timeout) => Err(io::ErrorKind::TimedOut.into()),
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidInput, e.to_string())),
        }
    }
    /// The hubs the platform's browse sees right now (LAN addresses).
    fn discovered(&self) -> Vec<SocketAddr>;
    /// Starts or stops the browse (only while syncing in the foreground).
    fn browse(&self, on: bool);
    /// Shows the scanner and waits for a code, `cancel`, or the timeout.
    fn scan(&self, cancel: &AtomicBool) -> Result<String, ScanError>;
    /// Asks for background time (a `Bye` and the current batch); 0: none.
    fn begin_bg(&self) -> u64 {
        0
    }
    fn end_bg(&self, _token: u64) {}
    /// Whether this phone can run a final pass itself (the live tier).
    fn capable(&self) -> bool;
    /// A recording is running: changes are not sent at once (the periodic
    /// pass still is).
    fn recording(&self) -> bool {
        false
    }
}

/// A boxed stream as the session layer's [`ByteStream`].
struct DynStream(Box<dyn ByteStream>);

impl Read for DynStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for DynStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl ByteStream for DynStream {
    fn set_io_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_io_timeout(timeout)
    }
}

#[derive(Default)]
struct SpokeState {
    /// The app is in the foreground.
    active: bool,
    browsing: bool,
    scanning: Option<Arc<AtomicBool>>,
    /// Addresses from the pairing code (memory only).
    qr_addrs: Vec<SocketAddr>,
    run_now: bool,
    /// A command is using the hub: the loop keeps no connection.
    hold: bool,
    /// The loop has a connection (or a store) right now.
    holding: bool,
    /// The background task begun when the app left the foreground.
    bg: u64,
    reported_down: bool,
    last_pending: Option<u32>,
}

/// The phone's loop state (idle on the desktop).
#[derive(Default)]
pub(super) struct Shared {
    st: Mutex<SpokeState>,
    wake: (Mutex<bool>, Condvar),
    thread: Mutex<Option<JoinHandle<()>>>,
}

/// How long to wait before the next connection attempt.
#[derive(Default)]
struct Backoff {
    fails: u32,
    next: Option<Instant>,
}

impl Backoff {
    fn ready(&self) -> bool {
        self.next.is_none_or(|t| Instant::now() >= t)
    }

    fn fail(&mut self) {
        self.fails = self.fails.saturating_add(1);
        let secs = match self.fails {
            1 => 2,
            2 => 5,
            3 => 10,
            _ => 30,
        };
        self.next = Some(Instant::now() + Duration::from_secs(secs));
    }

    fn reset(&mut self) {
        *self = Backoff::default();
    }
}

type Net = SpokeSession<NoiseTransport<DynStream>>;

struct Conn {
    session: Net,
    hub: String,
    store: Arc<Store>,
    last_run: Instant,
    last_ping: Instant,
    /// What was waiting to go after the last pass; more than this is news.
    baseline: u32,
}

/// Why no session could be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Down {
    Unreachable,
    Refused,
    Other(SyncErrorCode),
}

fn is_local_network_denied(e: &io::Error) -> bool {
    // iOS answers a connect without the Local Network permission with "no route
    // to host" (EHOSTUNREACH, 65 on Apple), or EPERM.
    e.kind() == io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(65)
}

fn ms_to_ns(ms: i64) -> i64 {
    ms.saturating_mul(1_000_000)
}

// --- the meetings waiting for the desktop ----------------------------------

/// The meetings recorded or imported for the desktop that have not been
/// offered to it yet.
pub fn desktop_pending(store: &Store) -> Vec<String> {
    store
        .get_setting(DESKTOP_PENDING_KEY)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn set_desktop_pending(store: &Store, list: &[String]) -> Result<(), String> {
    store
        .set_setting(DESKTOP_PENDING_KEY, &serde_json::json!(list))
        .map_err(|e| e.to_string())
}

/// Lists `meeting` as waiting for the desktop (a recording stopped, or an
/// import finished, with the `Desktop` target).
pub fn desktop_pending_add(store: &Store, meeting: &str) -> Result<(), String> {
    let mut list = desktop_pending(store);
    if !list.iter().any(|g| g == meeting) {
        list.push(meeting.to_string());
        set_desktop_pending(store, &list)?;
    }
    Ok(())
}

fn desktop_pending_remove(store: &Store, meeting: &str) {
    let mut list = desktop_pending(store);
    let before = list.len();
    list.retain(|g| g != meeting);
    if list.len() != before
        && let Err(e) = set_desktop_pending(store, &list)
    {
        log::warn!("desktop list not updated: {e}");
    }
}

// --- what the UI shows of a meeting -----------------------------------------

/// Whether a computer is paired with this phone (the `Desktop` target works).
pub fn has_hub(store: &Store) -> bool {
    store.devices().is_ok_and(|d| {
        d.iter().any(|d| {
            d.role == ghi_store::sync::devices::DeviceRole::Hub
                && d.state == StoreDeviceState::Paired
        })
    })
}

/// A lease this phone granted, as the UI sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeaseView {
    pub state: GrantorState,
    /// 0..=100.
    pub percent: u8,
}

/// Where a meeting stands with the paired computer (the chip, the audio bar
/// and the read-only transcript are derived from it).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SyncView {
    /// The paired computer's name.
    pub device: Option<String>,
    /// The newest lease this phone granted for the meeting.
    pub lease: Option<LeaseView>,
    /// Recorded for the desktop and not handed over yet.
    pub pending: bool,
    /// The computer has the meeting and nothing of it is waiting to go.
    pub synced: bool,
}

/// The phone's view of `meeting` against its paired hub (all empty without
/// one).
pub fn meeting_sync_view(store: &Store, meeting: &str) -> SyncView {
    let hub = store
        .devices()
        .ok()
        .and_then(|d| {
            d.into_iter()
                .find(|d| d.role == ghi_store::sync::devices::DeviceRole::Hub)
        })
        .filter(|d| d.state == StoreDeviceState::Paired);
    let Some(hub) = hub else {
        return SyncView::default();
    };
    let lease = store
        .leases_for_meeting(meeting)
        .unwrap_or_default()
        .into_iter()
        .filter(|l| l.role == LeaseRole::Grantor)
        // The newest lease is the one with the highest epoch.
        .max_by_key(|l| (l.epoch, l.state == "self_taken"))
        .and_then(|l| {
            Some(LeaseView {
                state: Grantor.state_of(&SystemClock, &l)?,
                percent: (l.progress.clamp(0.0, 1.0) * 100.0).round() as u8,
            })
        });
    let known = SyncStore::peer_meetings(store, &hub.gid)
        .map(|m| m.iter().any(|g| g == meeting))
        .unwrap_or(false);
    let dirty = SyncStore::sync_dirty(store, "meeting", meeting).unwrap_or(false);
    SyncView {
        device: Some(hub.name),
        lease,
        pending: desktop_pending(store).iter().any(|g| g == meeting),
        synced: known && !dirty,
    }
}

impl SyncView {
    /// The lease is open on the computer (it writes; the phone reads).
    pub fn lease_open(&self) -> Option<(&str, u8)> {
        let l = self.lease?;
        matches!(l.state, GrantorState::Granted | GrantorState::Revoking)
            .then(|| (self.device.as_deref().unwrap_or_default(), l.percent))
    }
}

// --- the service's phone side -----------------------------------------------

impl SyncService {
    fn link(&self) -> Result<&Arc<dyn SpokeLink>, String> {
        self.cfg
            .link
            .as_ref()
            .ok_or_else(|| ERR_NOT_SPOKE.to_string())
    }

    fn spoke_st(&self) -> MutexGuard<'_, SpokeState> {
        lock(&self.spoke.st)
    }

    pub(super) fn spoke_wake_up(&self) {
        *lock(&self.spoke.wake.0) = true;
        self.spoke.wake.1.notify_all();
    }

    /// The loop looks at once: a pass, whatever the timers say.
    pub(super) fn spoke_run_now(&self) {
        self.spoke_st().run_now = true;
        self.spoke_wake_up();
    }

    pub(super) fn spoke_start(self: &Arc<Self>) {
        let mut slot = lock(&self.spoke.thread);
        if slot.is_some() {
            return;
        }
        let weak = Arc::downgrade(self);
        *slot = std::thread::Builder::new()
            .name("ghi-sync-spoke".into())
            .spawn(move || spoke_loop(weak))
            .map_err(|e| log::warn!("sync session thread did not start: {e}"))
            .ok();
    }

    pub(super) fn spoke_stop(&self) {
        self.spoke_wake_up();
        if let Some(t) = lock(&self.spoke.thread).take()
            && t.thread().id() != std::thread::current().id()
        {
            let _ = t.join();
        }
    }

    /// Waits (bounded) until the loop holds no connection and no store.
    pub(super) fn spoke_wait_idle(&self) {
        let until = Instant::now() + HOLD_WAIT;
        while self.spoke_st().holding && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The app moved to the foreground or the background. Leaving ends the
    /// session with `Bye` inside a background task, which lets the pass in
    /// flight finish (the loop ends the task once it has said goodbye).
    pub fn set_app_active(&self, active: bool) {
        let Some(link) = self.cfg.link.as_ref() else {
            return;
        };
        let begun = if active { 0 } else { link.begin_bg() };
        {
            let mut st = self.spoke_st();
            st.active = active;
            if begun != 0 {
                let old = std::mem::replace(&mut st.bg, begun);
                if old != 0 {
                    link.end_bg(old);
                }
            }
        }
        self.spoke_wake_up();
    }

    // --- pairing ----------------------------------------------------------

    /// Phone: shows the scanner, waits for the hub's code and pairs with it.
    /// Returns when paired (the [`SyncEvent::Paired`] is emitted too), when the
    /// scan was cancelled, or with the code of what went wrong (see the
    /// module docs).
    pub fn pair_scan(self: &Arc<Self>) -> Result<(), String> {
        let link = self.link()?.clone();
        let store = self.store()?;
        if !Self::enabled(&store) {
            return Err(ERR_OFF.into());
        }
        if store
            .devices()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|d| d.role == ghi_store::sync::devices::DeviceRole::Hub)
        {
            return Err(ERR_ALREADY_PAIRED.into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(old) = self.spoke_st().scanning.replace(cancel.clone()) {
            old.store(true, Ordering::Release);
        }
        let scanned = link.scan(&cancel);
        {
            let mut st = self.spoke_st();
            if st
                .scanning
                .as_ref()
                .is_some_and(|c| Arc::ptr_eq(c, &cancel))
            {
                st.scanning = None;
            }
        }
        let text = match scanned {
            Ok(t) => Zeroizing::new(t),
            Err(ScanError::Denied | ScanError::Unavailable) => return Err(SCAN_CAMERA_OFF.into()),
            Err(ScanError::Cancelled | ScanError::Timeout) => return Ok(()),
        };
        self.pair_with_code(&store, &link, &text)
    }

    /// Closes the scanner (the user left the screen).
    pub fn pair_scan_stop(&self) {
        if let Some(c) = self.spoke_st().scanning.take() {
            c.store(true, Ordering::Release);
        }
    }

    fn pair_with_code(
        self: &Arc<Self>,
        store: &Arc<Store>,
        link: &Arc<dyn SpokeLink>,
        text: &str,
    ) -> Result<(), String> {
        let code = qr::parse(text).map_err(|_| SCAN_INVALID.to_string())?;
        let identity = self.identity(store)?;
        let mut local_net = false;
        let mut refused = false;
        for addr in &code.addrs {
            let stream = match link.connect(*addr) {
                Ok(s) => s,
                Err(e) => {
                    local_net |= is_local_network_denied(&e);
                    continue;
                }
            };
            match pair_over(
                store.as_ref(),
                &identity,
                &code,
                DynStream(stream),
                &self.cfg.device_name,
                &self.cfg.platform,
            ) {
                Ok(p) => {
                    store
                        .touch_device(&p.device_gid, Some(&addr.to_string()))
                        .map_err(|e| e.to_string())?;
                    {
                        let mut st = self.spoke_st();
                        st.qr_addrs = code.addrs.clone();
                        st.reported_down = false;
                    }
                    self.on_paired(&p.device_gid);
                    self.refresh_known(None);
                    self.spoke_run_now();
                    return Ok(());
                }
                // A hub that closes the handshake: the code is not (or no
                // longer) accepted.
                Err(SyncError::Closed | SyncError::Noise(_) | SyncError::Timeout) => {
                    refused = true;
                }
                Err(SyncError::Peer(ghi_sync::wire::ErrorCode::UpgradeRequired)) => {
                    return Err(SCAN_UPGRADE.into());
                }
                Err(SyncError::Peer(ghi_sync::wire::ErrorCode::StorageFull)) => {
                    self.report_error(SyncErrorCode::StorageFull);
                    return Ok(());
                }
                // The key in the code is not the one that answered.
                Err(SyncError::Wire(_)) => return Err(SCAN_INVALID.into()),
                Err(SyncError::Store(_)) => {
                    self.report_error(SyncErrorCode::Internal);
                    return Ok(());
                }
                Err(_) => {}
            }
        }
        if refused {
            return Err(SCAN_EXPIRED.into());
        }
        if local_net {
            return Err(SCAN_LOCAL_NETWORK.into());
        }
        self.report_error(SyncErrorCode::Unreachable);
        Ok(())
    }

    fn report_error(&self, code: SyncErrorCode) {
        lock(&self.state).last_error = Some(code);
        self.emit(SyncEvent::Error { code });
    }

    // --- leases the phone grants ----------------------------------------

    /// "Process on this phone now": takes a granted lease back. The revoke is
    /// sent in the next pass; the desktop answers `Revoked` (the phone then
    /// runs the final pass at `epoch + 1`) or `AlreadyDone` (nothing to do).
    pub fn lease_revoke(self: &Arc<Self>, meeting: &str) -> Result<(), String> {
        let link = self.link()?;
        if !link.capable() {
            return Err(ERR_NOT_CAPABLE.into());
        }
        let store = self.store()?;
        Grantor
            .revoke(store.as_ref(), meeting)
            .map_err(|_| ERR_NO_LEASE.to_string())?;
        self.spoke_run_now();
        Ok(())
    }

    /// Runs the final pass of a meeting here, at the epoch after the lease's
    /// (the lease was revoked, or its desktop stayed away too long). A phone
    /// that cannot run one keeps the meeting waiting for the desktop.
    fn self_take(&self, store: &Store, take: &TakeBack) {
        if !take.kinds.iter().any(|k| k == FINAL_PASS_JOB) {
            return;
        }
        let jobs = store
            .jobs_for_meeting(&take.meeting_gid)
            .unwrap_or_default();
        let queued = jobs.iter().any(|j| {
            j.kind == FINAL_PASS_JOB
                && j.payload.get("epoch").and_then(serde_json::Value::as_i64) == Some(take.epoch)
        });
        if !queued {
            if let Err(e) = store.set_meeting_status(&take.meeting_gid, "processing") {
                log::warn!("self-take: status: {e}");
            }
            if let Err(e) = store.enqueue_job(
                Some(&take.meeting_gid),
                FINAL_PASS_JOB,
                JOB_PAYLOAD_VERSION,
                &serde_json::json!({"epoch": take.epoch}),
            ) {
                log::warn!("self-take: the final pass was not queued: {e}");
                return;
            }
        }
        desktop_pending_remove(store, &take.meeting_gid);
        self.core.notify_jobs();
    }

    /// Offers a lease for every meeting waiting for the desktop whose rows, key
    /// and audio the hub has acked. True when an offer was made (it is sent in
    /// the next pass).
    fn offer_pending(&self, store: &Arc<Store>, hub: &str) -> bool {
        let hours = (self.cfg.offline_hours)().clamp(1, 168);
        let ttl_ms = i64::from(hours) * 3_600_000;
        let mut offered = false;
        for gid in desktop_pending(store) {
            match store.get_meeting(&gid) {
                Ok(_) => {}
                Err(ghi_store::StoreError::NotFound { .. }) => {
                    desktop_pending_remove(store, &gid);
                    continue;
                }
                Err(_) => continue,
            }
            let leased = store
                .leases_for_meeting(&gid)
                .unwrap_or_default()
                .iter()
                .any(|l| l.role == LeaseRole::Grantor);
            if leased {
                desktop_pending_remove(store, &gid);
                continue;
            }
            let kinds = [FINAL_PASS_JOB.to_string(), NOTES_FINAL_JOB.to_string()];
            // Not everything is on the hub yet: the next pass tries again.
            if let Ok(req) = Grantor.offer(store.as_ref(), hub, &gid, &kinds, ttl_ms) {
                // The hours without the desktop count from now.
                let clock = SystemClock;
                let g = i64::try_from(clock.now_cont_ns())
                    .unwrap_or(i64::MAX)
                    .saturating_add(ms_to_ns(ttl_ms.saturating_add(ghi_sync::lease::GRACE_MS)));
                if let Err(e) = SyncStore::lease_renew(
                    store.as_ref(),
                    &req.job_uuid,
                    ttl_ms,
                    g,
                    &clock.boot_id(),
                ) {
                    log::warn!("lease deadline not set: {e}");
                }
                desktop_pending_remove(store, &gid);
                offered = true;
            }
        }
        offered
    }

    /// Takes back a lease whose hub stayed away past its deadline plus the
    /// grace, on a phone that can run the pass.
    fn poll_leases(&self, store: &Arc<Store>) {
        let capable = self.cfg.link.as_ref().is_some_and(|l| l.capable());
        match Grantor.poll(store.as_ref(), &SystemClock, 0, capable) {
            Ok(actions) => {
                for GrantorAction::SelfTake(t) in actions {
                    self.self_take(store, &t);
                }
            }
            Err(e) => log::warn!("lease check: {e}"),
        }
    }

    // --- talking to the hub outside the loop ------------------------------

    /// Delivers `ctl` to the hub when it can be reached (best effort, one
    /// attempt per address); what the control does on this side happens only
    /// if the hub took it.
    pub(super) fn spoke_tell_hub(self: &Arc<Self>, store: &Arc<Store>, ctl: Control) {
        self.spoke_st().hold = true;
        self.spoke_wake_up();
        self.spoke_wait_idle();
        match self.spoke_open(store) {
            Ok(mut c) => {
                if let Err(e) = c.session.send_control(ctl) {
                    log::info!("the computer was not told: {e}");
                }
            }
            Err(d) => log::info!("the computer was not told: {d:?}"),
        }
        self.spoke_st().hold = false;
        self.spoke_wake_up();
    }

    // --- opening a session --------------------------------------------------

    fn candidates(&self, hub: &Device) -> Vec<SocketAddr> {
        let mut out: Vec<SocketAddr> = Vec::new();
        let mut push = |a: SocketAddr| {
            if ghi_net::is_lan(a.ip()) && !out.contains(&a) {
                out.push(a);
            }
        };
        if let Some(a) = hub.last_addr.as_deref().and_then(|a| a.parse().ok()) {
            push(a);
        }
        for a in self.spoke_st().qr_addrs.clone() {
            push(a);
        }
        if let Some(l) = &self.cfg.link {
            for a in l.discovered() {
                push(a);
            }
        }
        out
    }

    fn spoke_open(self: &Arc<Self>, store: &Arc<Store>) -> Result<Conn, Down> {
        let link = self
            .cfg
            .link
            .clone()
            .ok_or(Down::Other(SyncErrorCode::Internal))?;
        let identity = self
            .identity(store)
            .map_err(|_| Down::Other(SyncErrorCode::Internal))?;
        let sync_store: &dyn SyncStore = store.as_ref();
        let hub = paired_hub(sync_store).map_err(|_| Down::Other(SyncErrorCode::Internal))?;
        let mut refused = false;
        for addr in self.candidates(&hub) {
            let Ok(stream) = link.connect(addr) else {
                continue;
            };
            match initiate_hub(sync_store, &identity, DynStream(stream)) {
                Ok((transport, hub)) => {
                    let _ = store.touch_device(&hub.gid, Some(&addr.to_string()));
                    let weak = Arc::downgrade(self);
                    let watched: Arc<dyn SyncStore> = Arc::new(Watched {
                        store: store.clone(),
                        seen: Arc::new(move |s| {
                            if let Some(me) = weak.upgrade() {
                                me.on_seen(s);
                            }
                        }),
                    });
                    let session = SpokeSession::new(
                        watched,
                        Arc::new(SystemClock),
                        transport,
                        hub.gid.clone(),
                    );
                    return Ok(Conn {
                        session,
                        hub: hub.gid,
                        store: store.clone(),
                        last_run: Instant::now(),
                        last_ping: Instant::now(),
                        baseline: pending_changes(store),
                    });
                }
                // A hub that knows no such device closes without a word.
                Err(SyncError::Closed | SyncError::Noise(_)) => refused = true,
                Err(_) => {}
            }
        }
        Err(if refused {
            Down::Refused
        } else {
            Down::Unreachable
        })
    }

    /// A failed open: tell the UI once and back off. The pin always stays.
    fn spoke_down(self: &Arc<Self>, down: Down, back: &mut Backoff) {
        back.fail();
        match down {
            Down::Unreachable => {
                let first = !std::mem::replace(&mut self.spoke_st().reported_down, true);
                if first {
                    self.report_error(SyncErrorCode::Unreachable);
                }
            }
            Down::Other(code) => self.report_error(code),
            // Anything on the LAN can close a handshake (another Ghira
            // computer, a full hub, a reset): that never drops the pin. A
            // hub that unpaired this phone says so with `Control::Unpair`
            // inside a session; the user can unpair by hand.
            Down::Refused => {
                let first = !std::mem::replace(&mut self.spoke_st().reported_down, true);
                if first {
                    self.report_error(SyncErrorCode::Refused);
                }
            }
        }
    }

    // --- one tick of the loop -----------------------------------------------

    /// The store, when a session may exist now: the app is in the foreground
    /// and unlocked, sync is on and a hub is paired.
    fn spoke_gate(&self) -> Option<Arc<Store>> {
        {
            let st = self.spoke_st();
            if !st.active || st.hold {
                return None;
            }
        }
        if self.shutdown.load(Ordering::Acquire)
            || lock(&self.state).closed
            || self.core.locked()
            || self.core.open_problem().is_some()
        {
            return None;
        }
        let store = self.core.store_even_locked().ok()?;
        if !Self::enabled(&store) {
            return None;
        }
        store
            .devices()
            .ok()?
            .iter()
            .any(|d| d.role == ghi_store::sync::devices::DeviceRole::Hub)
            .then_some(store)
    }

    /// Ends the session politely and gives the background time back.
    fn spoke_close(&self, conn: &mut Option<Conn>) {
        if let Some(mut c) = conn.take() {
            let _ = c.session.bye();
        }
        let tok = std::mem::take(&mut self.spoke_st().bg);
        if tok != 0
            && let Some(l) = &self.cfg.link
        {
            l.end_bg(tok);
        }
        self.spoke_st().holding = false;
    }

    fn spoke_tick(
        self: &Arc<Self>,
        conn: &mut Option<Conn>,
        back: &mut Backoff,
        polled: &mut Instant,
    ) {
        let Some(store) = self.spoke_gate() else {
            self.spoke_close(conn);
            self.spoke_browse(false);
            return;
        };
        self.spoke_browse(true);
        if polled.elapsed() >= POLL_EVERY {
            *polled = Instant::now();
            self.poll_leases(&store);
        }
        if conn.is_none() {
            if !back.ready() {
                return;
            }
            match self.spoke_open(&store) {
                Ok(c) => {
                    back.reset();
                    {
                        let mut st = self.spoke_st();
                        st.reported_down = false;
                        st.holding = true;
                        st.run_now = true;
                    }
                    *conn = Some(c);
                }
                Err(d) => {
                    self.spoke_down(d, back);
                    return;
                }
            }
        }
        let Some(c) = conn.as_mut() else { return };
        let link = self.cfg.link.clone();
        let lease_open = store.leases_open().is_ok_and(|l| {
            l.iter().any(|l| {
                l.role == LeaseRole::Grantor && matches!(l.state.as_str(), "granted" | "running")
            })
        });
        let every = if lease_open { LEASE_PASS } else { IDLE_PASS };
        let now_run = std::mem::take(&mut self.spoke_st().run_now);
        let changed = !link.as_ref().is_some_and(|l| l.recording())
            && c.last_run.elapsed() >= CHANGE_DEBOUNCE
            && pending_changes(&store) > c.baseline;
        let result = if now_run || changed || c.last_run.elapsed() >= every {
            self.spoke_pass(c)
        } else if c.last_ping.elapsed() >= PING_EVERY {
            c.last_ping = Instant::now();
            match c.session.ping() {
                Ok(true) => self.spoke_pass(c),
                Ok(false) if c.session.report().closed_by.is_some() => {
                    let closed = c.session.report().closed_by;
                    self.refresh_known(closed);
                    Err(())
                }
                Ok(false) => Ok(()),
                Err(e) => {
                    self.session_failed(&e);
                    Err(())
                }
            }
        } else {
            Ok(())
        };
        if result.is_err() {
            *conn = None;
            let mut st = self.spoke_st();
            st.holding = false;
            back.fail();
        }
    }

    fn spoke_browse(&self, on: bool) {
        let Some(link) = &self.cfg.link else { return };
        let changed = {
            let mut st = self.spoke_st();
            std::mem::replace(&mut st.browsing, on) != on
        };
        if changed {
            link.browse(on);
        }
    }

    /// A failed pass: the code the UI words (a dropped connection is the
    /// computer being away).
    fn session_failed(&self, e: &SyncError) {
        let code = error_code(e).unwrap_or(SyncErrorCode::Unreachable);
        let first = {
            let mut st = self.spoke_st();
            code != SyncErrorCode::Unreachable || !std::mem::replace(&mut st.reported_down, true)
        };
        if first {
            self.report_error(code);
        }
    }

    /// One full pass on the open session. `Err`: the connection is done.
    fn spoke_pass(self: &Arc<Self>, c: &mut Conn) -> Result<(), ()> {
        let store = c.store.clone();
        // A wipe the user asked for goes first.
        if store
            .device(&c.hub)
            .ok()
            .flatten()
            .is_some_and(|d| d.state == StoreDeviceState::WipePending)
        {
            return match c.session.send_control(Control::Wipe {
                reason: "wipe".into(),
            }) {
                Ok(outcome) => {
                    self.refresh_known(Some(outcome));
                    Err(())
                }
                Err(e) => {
                    self.session_failed(&e);
                    Err(())
                }
            };
        }
        let report = match c.session.run_once() {
            Ok(r) => r,
            Err(e) => {
                self.session_failed(&e);
                return Err(());
            }
        };
        if let Some(outcome) = report.closed_by {
            self.refresh_known(Some(outcome));
            return Err(());
        }
        {
            let mut st = self.spoke_st();
            st.reported_down = false;
        }
        lock(&self.state).last_error = None;
        self.on_pass(&store, &c.hub, &report);
        c.baseline = pending_changes(&store);
        c.last_run = Instant::now();
        c.last_ping = Instant::now();
        Ok(())
    }

    /// What a finished pass changes on this side.
    fn on_pass(self: &Arc<Self>, store: &Arc<Store>, hub: &str, rep: &SessionReport) {
        for info in &rep.lease_infos {
            let lease = store
                .leases_for_meeting(&info.meeting_gid)
                .unwrap_or_default()
                .into_iter()
                .find(|l| l.role == LeaseRole::Grantor && l.job_uuid == info.job_uuid);
            if let Some(l) = lease
                && let Err(e) = store.lease_set_progress(&l.job_uuid, info.progress)
            {
                log::warn!("lease progress: {e}");
            }
        }
        for t in &rep.take_back {
            self.self_take(store, t);
        }
        self.poll_leases(store);
        // The hub has this meeting's rows, key and audio: hand it over now.
        if self.offer_pending(store, hub) {
            self.spoke_st().run_now = true;
        }
        if rep.needs_confirm > 0 {
            self.ask_mass_delete(hub, rep.needs_confirm);
        }
        self.refresh_known(None);
        let pending = pending_changes(store);
        let changed = rep.rows_pulled
            + rep.tombs_pulled
            + rep.rows_pushed
            + rep.tombs_pushed
            + rep.tracks_sent
            + rep.lease_infos.len()
            + rep.take_back.len()
            > 0;
        let moved = self.spoke_st().last_pending.replace(pending) != Some(pending);
        if changed || moved {
            self.emit(SyncEvent::Progress { pending });
        }
    }
}

fn spoke_loop(weak: Weak<SyncService>) {
    let mut conn: Option<Conn> = None;
    let mut back = Backoff::default();
    let mut polled = Instant::now();
    loop {
        let Some(me) = weak.upgrade() else { return };
        if me.shutdown.load(Ordering::Acquire) {
            me.spoke_close(&mut conn);
            return;
        }
        me.spoke_tick(&mut conn, &mut back, &mut polled);
        let (flag, cv) = &me.spoke.wake;
        let mut woken = lock(flag);
        if !*woken {
            woken = cv
                .wait_timeout(woken, LOOP_TICK)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        *woken = false;
    }
}
