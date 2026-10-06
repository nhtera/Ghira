// SPDX-License-Identifier: Apache-2.0
//! The app's LAN sync service (phase 15, slice 15-J; doc 07 §2.4, §3.5, §7.9,
//! §8): one [`SyncService`] per process, managed by the app beside
//! [`Core`]. It owns the hub's listener, the mDNS advertisement and the
//! pairing window, serves paired phones with [`ghi_sync`]'s `HubNode`, turns
//! the leases a phone grants into local jobs, and emits [`SyncEvent`]s
//! (codes and counts only).
//!
//! **Lifetimes.** The listener, the advertisement and the pairing window
//! exist only while `(sync on and at least one device paired and the app
//! unlocked)` or `the pairing sheet is open`. [`SyncService::reconcile`]
//! makes that true; it runs on every change that matters (a toggle, a
//! pairing, an unpair, the app lock via [`Core::set_locked`]) and every
//! 10 s, when the listener also follows the machine's addresses
//! (`Listener::rebind`). The store is the app's own ([`Core::store_even_locked`]):
//! nothing here opens a second one.
//!
//! **Roles.** The desktop is the [`Role::Hub`]. The phone runs the same
//! service as the [`Role::Spoke`] (`sync_spoke.rs`): it never listens, scans
//! the hub's code, keeps one session to the hub while the app is in the
//! foreground and hands it final passes under leases. The commands that
//! don't need a listener work for both.
//!
//! **One path for "Delete everything".** [`SyncService::delete_everywhere_prepare`]
//! queues `Wipe` for every paired device and waits (bounded, and only for
//! devices seen a moment ago) before `Core::delete_everything_with` removes
//! the data and the identity.
//!
//! Nothing in this file logs a pairing code, a PSK or a key.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ghi_core::jobs::{Fence, FenceAt, JobLease};
use ghi_core::notes_job::NOTES_FINAL_JOB;
use ghi_core::session::{FINAL_PASS_JOB, JOB_PAYLOAD_VERSION};
use ghi_net::lan::{self, LanStream, Limits, Listener};
use ghi_store::jobs::Job;
use ghi_store::keys::secrets::SecretStore;
use ghi_store::store::Store;
use ghi_store::sync::apply::{ApplyOutcome, ApplyResult, TombResult};
use ghi_store::sync::devices::{Device, DeviceState as StoreDeviceState, NewDevice};
use ghi_store::sync::feed::{ChangeBatch, TombBatch};
use ghi_store::sync::leases::{Lease, LeaseRole};
use ghi_store::sync::records::{Record, SettingRec, SyncTombstone};
use ghi_store::sync::wipe::WipeReport;
use ghi_sync::SyncStore;
use ghi_sync::audio::{OfferResult, TrackInfo};
use ghi_sync::clock::{Clock, SystemClock};
use ghi_sync::control::ControlOutcome;
use ghi_sync::identity::Identity;
use ghi_sync::service::{HubNode, Served};
use ghi_sync::session::SessionReport;
use ghi_sync::transport::ByteStream;
use ghi_sync::wire::{self, TrackOffer};
use ghi_sync::{SyncError, qr};
use zeroize::Zeroizing;

pub use self::spoke::{ScanError, SpokeLink};
use crate::core::Core;
use crate::sync_cmd::{
    ConflictCopy, ConflictTarget, DeleteEverywhereState, DeleteEverywhereStatus, DevicePlatform,
    DeviceRow, DeviceState, PAIR_CODE_TTL_MS, PairOffer, SyncErrorCode, SyncEvent, SyncStatus,
};

type StoreResult<T> = ghi_store::Result<T>;

/// The store setting that switches sync on (device-local; not synced).
pub const ENABLED_KEY: &str = "sync.enabled";
/// The store setting holding the listener's port (chosen once, then kept).
pub const PORT_KEY: &str = "sync.port";

/// The listener follows the machine's addresses this often.
pub const REBIND_EVERY: Duration = Duration::from_secs(10);
/// How long a pairing code works.
pub const PAIR_TTL: Duration = Duration::from_secs(120);
/// Connections from one address at the same time. A device has one session
/// (a new one replaces the old as soon as `Hello` names the device); this is
/// only the bound for connections that have not said who they are yet (the
/// pool of `ghi_net::lan::MAX_SESSIONS` is the cap over all of them).
const MAX_PER_ADDRESS: usize = 4;
/// How long "Delete everything" waits for devices to take their `Wipe`.
pub const WIPE_WAIT: Duration = Duration::from_secs(120);
/// How long an unpair waits for the device to come and take it (doc 07 §3.5);
/// then the pin goes anyway.
pub const UNPAIR_WAIT: Duration = Duration::from_secs(7 * 24 * 3600);
/// A device seen within this long is reachable for the wipe's wait.
pub const REACHABLE_WITHIN: Duration = Duration::from_secs(90);
/// A refused mass delete is not asked about again for this long.
const DECLINE_QUIET: Duration = Duration::from_secs(10 * 60);
/// A runner's commit keeps this margin before the lease's deadline (doc 07 §8).
pub const COMMIT_MARGIN_MS: i64 = 60_000;
/// A stopping service waits this long for its sessions to end.
const STOP_WAIT: Duration = Duration::from_secs(3);
/// A blocked read looks at the stop flag this often.
const READ_POLL: Duration = Duration::from_millis(250);

/// Error codes of the commands (the UI turns them into words).
pub const ERR_OFF: &str = "sync_off";
pub const ERR_NO_NETWORK: &str = "sync_no_network";
pub const ERR_UNKNOWN_DEVICE: &str = "unknown_device";
pub const ERR_UNKNOWN_CONFLICT: &str = "unknown_conflict";
pub const ERR_NOTHING_TO_CONFIRM: &str = "nothing_to_confirm";
pub const ERR_NOT_HUB: &str = "sync_not_available";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> i64 {
    SystemClock.wall_ms()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The desktop: listens, never connects out.
    Hub,
    /// The phone: connects to its hub (wired by the mobile app).
    Spoke,
}

/// What the app gives its service.
#[derive(Clone)]
pub struct SyncConfig {
    pub role: Role,
    /// The name a phone shows for this computer (sent in `PairAccept`).
    pub device_name: String,
    /// Where events go (the apps emit them as the typed `syncEvent`).
    pub emit: Arc<dyn Fn(SyncEvent) + Send + Sync>,
    /// After a session changed a synced setting (the settings cache has been
    /// dropped): the app applies what follows from a settings change.
    pub settings_changed: Arc<dyn Fn() + Send + Sync>,
    /// The addresses sync may use (default: the machine's private ones).
    pub addrs: Arc<dyn Fn() -> Vec<IpAddr> + Send + Sync>,
    /// Advertise over mDNS (needs the `mdns` feature).
    pub advertise: bool,
    /// A device seen within this long is waited for by "Delete everything".
    pub reachable_within: Duration,
    /// Spoke: the platform (camera, Bonjour, background time, the tier).
    pub link: Option<Arc<dyn SpokeLink>>,
    /// Spoke: what this device calls itself in a pairing (`ios`).
    pub platform: String,
    /// Spoke: hours without the desktop before this phone takes a final pass
    /// back (the setting "desktop offline for N hours").
    pub offline_hours: Arc<dyn Fn() -> u32 + Send + Sync>,
}

/// How long a desktop may stay away before a phone processes for itself.
pub const DEFAULT_OFFLINE_HOURS: u32 = 12;

impl SyncConfig {
    pub fn hub(
        device_name: impl Into<String>,
        emit: impl Fn(SyncEvent) + Send + Sync + 'static,
        settings_changed: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        SyncConfig {
            role: Role::Hub,
            device_name: device_name.into(),
            emit: Arc::new(emit),
            settings_changed: Arc::new(settings_changed),
            addrs: Arc::new(lan::lan_addrs),
            advertise: cfg!(feature = "mdns"),
            reachable_within: REACHABLE_WITHIN,
            link: None,
            platform: String::new(),
            offline_hours: Arc::new(|| DEFAULT_OFFLINE_HOURS),
        }
    }

    /// The phone: connects out to its hub through `link`.
    pub fn spoke(
        device_name: impl Into<String>,
        platform: impl Into<String>,
        link: Arc<dyn SpokeLink>,
        emit: impl Fn(SyncEvent) + Send + Sync + 'static,
        settings_changed: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        SyncConfig {
            role: Role::Spoke,
            device_name: device_name.into(),
            emit: Arc::new(emit),
            settings_changed: Arc::new(settings_changed),
            addrs: Arc::new(Vec::new),
            advertise: false,
            reachable_within: REACHABLE_WITHIN,
            link: Some(link),
            platform: platform.into(),
            offline_hours: Arc::new(|| DEFAULT_OFFLINE_HOURS),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
struct DeleteState {
    state: Option<DeleteEverywhereState>,
    waiting_for: Vec<String>,
}

struct Running {
    node: Arc<HubNode>,
    /// The listener's addresses (with the port), kept current by the accept
    /// loop; empty while the machine has no private network.
    addrs: Arc<Mutex<Vec<SocketAddr>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    pairing_until: Option<Instant>,
    /// "Delete everything" ran: nothing starts again in this process.
    closed: bool,
    last_error: Option<SyncErrorCode>,
    /// Paired devices as last seen: `gid -> (name, state)`, to tell what a
    /// session removed.
    known: Option<HashMap<String, (String, StoreDeviceState)>>,
    pending_confirm: Option<String>,
    declined: HashMap<String, Instant>,
    delete: DeleteState,
}

struct SessionEntry {
    kill: Arc<AtomicBool>,
    ip: IpAddr,
    /// The device this session serves, once `Hello` named it.
    device: Option<String>,
}

/// See the module docs.
pub struct SyncService {
    core: Arc<Core>,
    cfg: SyncConfig,
    state: Mutex<State>,
    /// Serializes [`SyncService::reconcile`].
    lifecycle: Mutex<()>,
    sessions: Mutex<HashMap<u64, SessionEntry>>,
    next_session: AtomicU64,
    wake: (Mutex<bool>, Condvar),
    shutdown: AtomicBool,
    tick: Mutex<Option<JoinHandle<()>>>,
    /// "Delete here only": the wipe's wait ends.
    skip_wipe: AtomicBool,
    /// Devices "Delete everything" moved to `wipe_pending` (and were not
    /// before): they go back to paired if the delete does not happen.
    queued_wipes: Mutex<Vec<String>>,
    /// The phone's side (idle on the desktop).
    spoke: spoke::Shared,
}

/// What the store wrapper saw applied.
enum Seen {
    Conflict(String),
    Settings,
    /// A phone offered a lease: its job may start now.
    Lease,
}

/// The store a hub node serves: the app's own, watched so a merge that made a
/// conflict copy or changed a synced setting is announced.
struct Watched {
    store: Arc<Store>,
    seen: Arc<dyn Fn(Seen) + Send + Sync>,
}

macro_rules! forward {
    ($(fn $name:ident(&self $(, $arg:ident: $ty:ty)*) -> $ret:ty;)*) => {
        $(fn $name(&self $(, $arg: $ty)*) -> $ret {
            SyncStore::$name(self.store.as_ref() $(, $arg)*)
        })*
    };
}

impl SyncStore for Watched {
    forward! {
        fn feed_id(&self) -> StoreResult<String>;
        fn regen_feed_id(&self) -> StoreResult<String>;
        fn device_gid(&self) -> StoreResult<String>;
        fn changes_since(&self, seq: i64, max: usize) -> StoreResult<ChangeBatch>;
        fn tombs_since(&self, seq: i64, max: usize) -> StoreResult<TombBatch>;
        fn relog_meeting(&self, meeting_gid: &str) -> StoreResult<()>;
        fn apply_tombs(&self, from_device: &str, tombs: &[SyncTombstone]) -> StoreResult<TombResult>;
        fn mark_clean(&self, gid: &str, lamport: i64) -> StoreResult<()>;
        fn retry_pending(&self) -> StoreResult<usize>;
        fn observe_lamport(&self, remote: i64) -> StoreResult<()>;
        fn devices(&self) -> StoreResult<Vec<Device>>;
        fn device(&self, gid: &str) -> StoreResult<Option<Device>>;
        fn device_by_key(&self, static_pub: &[u8; 32]) -> StoreResult<Option<Device>>;
        fn pin_device(&self, device: &NewDevice, pair_psk: &[u8; 32]) -> StoreResult<Device>;
        fn unpin_device(&self, gid: &str) -> StoreResult<()>;
        fn set_wipe_pending(&self, gid: &str) -> StoreResult<()>;
        fn set_unpair_pending(&self, gid: &str) -> StoreResult<()>;
        fn touch_device(&self, gid: &str, addr: Option<&str>) -> StoreResult<()>;
        fn pair_psk(&self, gid: &str) -> StoreResult<Zeroizing<[u8; 32]>>;
        fn set_cursors(&self, gid: &str, push_seq: i64, pull_feed_id: Option<&str>, pull_seq: i64) -> StoreResult<()>;
        fn meeting_dek_for_peer(&self, device_gid: &str, meeting_gid: &str) -> StoreResult<Option<Zeroizing<[u8; 32]>>>;
        fn mark_key_sent(&self, device_gid: &str, meeting_gid: &str) -> StoreResult<()>;
        fn accept_dek(&self, meeting_gid: &str, dek: &[u8; 32], from_device: &str) -> StoreResult<()>;
        fn peer_meetings(&self, device_gid: &str) -> StoreResult<Vec<String>>;
        fn lease_renew(&self, job_uuid: &str, ttl_ms: i64, deadline_cont_ns: i64, boot_id: &str) -> StoreResult<()>;
        fn lease_state(&self, job_uuid: &str) -> StoreResult<Option<Lease>>;
        fn lease_fence_ok(&self, job_uuid: &str, now_cont_ns: i64, boot_id: &str, margin_ms: i64) -> StoreResult<bool>;
        fn lease_any_open_for(&self, meeting_gid: &str) -> StoreResult<bool>;
        fn lease_transition(&self, job_uuid: &str, from: &[&str], to: &str) -> StoreResult<bool>;
        fn lease_revoke(&self, job_uuid: &str) -> StoreResult<bool>;
        fn leases_for_meeting(&self, meeting_gid: &str) -> StoreResult<Vec<Lease>>;
        fn leases_open(&self) -> StoreResult<Vec<Lease>>;
        fn mass_delete_confirmed(&self, device_gid: &str) -> StoreResult<bool>;
        fn set_mass_delete_confirmed(&self, device_gid: &str, confirmed: bool) -> StoreResult<()>;
        fn wipe_peer(&self, device_gid: &str) -> StoreResult<WipeReport>;
        fn put_synced(&self, key: &str, value_json: &str) -> StoreResult<()>;
        fn apply_synced(&self, rec: &SettingRec) -> StoreResult<()>;
        fn tracks_to_send(&self, device_gid: &str) -> StoreResult<Vec<TrackInfo>>;
        fn track_read_pages(&self, track_gid: &str, first: u64, max: usize) -> StoreResult<Vec<Vec<u8>>>;
        fn mark_track_sent(&self, device_gid: &str, track_gid: &str) -> StoreResult<()>;
        fn sync_dirty(&self, kind: &str, gid: &str) -> StoreResult<bool>;
        fn track_offer(&self, from_device: &str, offer: &TrackOffer) -> StoreResult<OfferResult>;
        fn track_push(&self, from_device: &str, track_gid: &str, prefix: &[u8], first: u64, records: &[Vec<u8>]) -> StoreResult<u64>;
    }

    fn lease_open(&self, lease: &Lease) -> StoreResult<Lease> {
        let opened = SyncStore::lease_open(self.store.as_ref(), lease)?;
        if lease.role == LeaseRole::Holder {
            (self.seen)(Seen::Lease);
        }
        Ok(opened)
    }

    fn apply_rows(&self, from_device: &str, rows: &[Record]) -> StoreResult<ApplyResult> {
        let res = SyncStore::apply_rows(self.store.as_ref(), from_device, rows)?;
        let mut meetings: Vec<String> = Vec::new();
        let mut settings = false;
        for (rec, (_, outcome)) in rows.iter().zip(&res.results) {
            match (rec, outcome) {
                (Record::Setting(_), ApplyOutcome::Accepted | ApplyOutcome::Merged) => {
                    settings = true;
                }
                (Record::ConflictCopy(_), ApplyOutcome::Accepted) => {
                    meetings.extend(rec.meeting_gid().map(str::to_string));
                }
                (_, ApplyOutcome::Merged) => {
                    meetings.push(rec.meeting_gid().unwrap_or_else(|| rec.gid()).to_string());
                }
                _ => {}
            }
        }
        meetings.sort();
        meetings.dedup();
        for m in meetings {
            (self.seen)(Seen::Conflict(m));
        }
        if settings {
            (self.seen)(Seen::Settings);
        }
        Ok(res)
    }
}

/// A hub connection whose reads give way to a stop flag: a quit, the app
/// lock or "Delete everything" ends sessions without waiting out a silent
/// peer, so the store is let go promptly.
struct Killable {
    inner: LanStream,
    kill: Arc<AtomicBool>,
    /// The read limit the session asked for (`None`: none).
    read_limit: Option<Duration>,
}

impl Killable {
    fn new(inner: LanStream, kill: Arc<AtomicBool>) -> io::Result<Self> {
        inner.set_read_timeout(Some(READ_POLL))?;
        Ok(Killable {
            inner,
            kill,
            read_limit: Some(lan::HANDSHAKE_TIMEOUT),
        })
    }
}

impl Read for Killable {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let start = Instant::now();
        loop {
            if self.kill.load(Ordering::Acquire) {
                return Err(io::ErrorKind::ConnectionAborted.into());
            }
            match self.inner.read(buf) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if self.read_limit.is_some_and(|l| start.elapsed() >= l) {
                        return Err(e);
                    }
                }
                other => return other,
            }
        }
    }
}

impl Write for Killable {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.kill.load(Ordering::Acquire) {
            return Err(io::ErrorKind::ConnectionAborted.into());
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl ByteStream for Killable {
    fn set_io_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        // Reads poll the stop flag every READ_POLL and keep the session's
        // own limit; writes use the limit as is.
        self.read_limit = timeout;
        self.inner.set_read_timeout(Some(READ_POLL))?;
        self.inner.set_write_timeout(timeout)
    }
}

/// Removes a session from the registry when its thread ends.
struct SessionGuard {
    svc: Arc<SyncService>,
    id: u64,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        lock(&self.svc.sessions).remove(&self.id);
    }
}

fn platform_of(d: &Device) -> DevicePlatform {
    let p = d.platform.to_ascii_lowercase();
    if p.starts_with("ios") || p.contains("iphone") || p.contains("ipad") {
        DevicePlatform::Ios
    } else if p.starts_with("win") {
        DevicePlatform::Windows
    } else {
        DevicePlatform::Mac
    }
}

fn device_row(d: &Device) -> DeviceRow {
    DeviceRow {
        gid: d.gid.clone(),
        name: d.name.clone(),
        platform: platform_of(d),
        state: match d.state {
            StoreDeviceState::Paired => DeviceState::Paired,
            StoreDeviceState::WipePending => DeviceState::WipePending,
            StoreDeviceState::UnpairPending => DeviceState::UnpairPending,
        },
        last_seen_ms: d.last_seen.map(|v| v as f64),
    }
}

/// The code of a failed session worth showing; a dropped connection or an
/// unknown device knocking is not (the phone shows its own side).
fn error_code(e: &SyncError) -> Option<SyncErrorCode> {
    match e {
        SyncError::Store(ghi_store::StoreError::Db(d))
            if d.to_string().contains("disk is full") =>
        {
            Some(SyncErrorCode::StorageFull)
        }
        SyncError::Store(_) => Some(SyncErrorCode::Internal),
        SyncError::Peer(ghi_sync::wire::ErrorCode::UpgradeRequired) => {
            Some(SyncErrorCode::UpgradeRequired)
        }
        SyncError::Peer(ghi_sync::wire::ErrorCode::StorageFull) => Some(SyncErrorCode::StorageFull),
        _ => None,
    }
}

fn code_text(code: SyncErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The fence the job runner asks for a leased job (doc 07 §8): the lease must
/// still be granted, in this boot, before its deadline; the commit keeps
/// [`COMMIT_MARGIN_MS`] to spare. Notes queued by the pass that closed the
/// lease run under it while its deadline holds. Jobs without a lease never
/// get here.
pub fn lease_fence(store: Arc<Store>) -> Fence {
    let clock = SystemClock;
    Arc::new(move |job: &Job, at: FenceAt| {
        let Some(l) = JobLease::from_payload(&job.payload) else {
            return true;
        };
        let margin = match at {
            FenceAt::Commit => COMMIT_MARGIN_MS,
            FenceAt::Claim | FenceAt::Checkpoint => 0,
        };
        let now = i64::try_from(clock.now_cont_ns()).unwrap_or(i64::MAX);
        let boot = clock.boot_id();
        match store.lease_fence_ok(&l.job_uuid, now, &boot, margin) {
            Ok(true) => true,
            Ok(false) if job.kind == NOTES_FINAL_JOB => {
                notes_after_the_pass(&store, &l, now, &boot, margin)
            }
            Ok(false) => false,
            Err(e) => {
                log::warn!("lease fence unreadable, job stops: {e}");
                false
            }
        }
    })
}

/// The final pass committed under the lease (`granted -> done`) and queued
/// the notes: they may still run until the lease's deadline.
fn notes_after_the_pass(
    store: &Store,
    l: &JobLease,
    now_ns: i64,
    boot: &str,
    margin_ms: i64,
) -> bool {
    match store.lease_state(&l.job_uuid) {
        Ok(Some(lease)) if lease.state == "done" => {
            lease.boot_id.as_deref() == Some(boot)
                && lease
                    .deadline_cont_ns
                    .is_some_and(|h| now_ns.saturating_add(margin_ms.saturating_mul(1_000_000)) < h)
        }
        _ => false,
    }
}

/// Drops the pairings of a store that was restored from an archive: the
/// restore gave it a new device gid and feed, so a pin made before can no
/// longer be used (C1), and the old identity goes too.
pub fn drop_restored_pins(store: &Store, secrets: &dyn SecretStore) -> Result<(), String> {
    for d in store.devices().map_err(|e| e.to_string())? {
        match store.unpin_device(&d.gid) {
            Ok(()) | Err(ghi_store::StoreError::NotFound { .. }) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Identity::delete(secrets).map_err(|e| e.to_string())
}

impl SyncService {
    /// Creates the service and registers it with `core` (which tells it about
    /// the app lock and about "Delete everything"). Call [`SyncService::start`]
    /// once the app is up.
    pub fn new(core: Arc<Core>, cfg: SyncConfig) -> Arc<Self> {
        let svc = Arc::new(SyncService {
            core: core.clone(),
            cfg,
            state: Mutex::new(State::default()),
            lifecycle: Mutex::new(()),
            sessions: Mutex::new(HashMap::new()),
            next_session: AtomicU64::new(1),
            wake: (Mutex::new(false), Condvar::new()),
            shutdown: AtomicBool::new(false),
            tick: Mutex::new(None),
            skip_wipe: AtomicBool::new(false),
            queued_wipes: Mutex::new(Vec::new()),
            spoke: spoke::Shared::default(),
        });
        core.attach_sync(&svc);
        svc
    }

    /// Starts the background thread that keeps the listener in step (every
    /// 10 s, and at once on [`SyncService::poke`]).
    pub fn start(self: &Arc<Self>) {
        let mut tick = lock(&self.tick);
        if tick.is_some() {
            return;
        }
        let weak = Arc::downgrade(self);
        *tick = std::thread::Builder::new()
            .name("ghi-sync".into())
            .spawn(move || tick_loop(weak))
            .map_err(|e| log::warn!("sync thread did not start: {e}"))
            .ok();
        drop(tick);
        if self.cfg.role == Role::Spoke {
            self.spoke_start();
        }
    }

    /// Stops listening and ends the sessions (the app is quitting).
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.poke();
        if let Some(t) = lock(&self.tick).take()
            && t.thread().id() != std::thread::current().id()
        {
            let _ = t.join();
        }
        self.spoke_stop();
        let _one = lock(&self.lifecycle);
        self.stop_running();
    }

    /// "Delete everything" is about to remove the data: the listener closes and
    /// the sessions end (they hold the store), and nothing starts again.
    pub fn stop_for_delete(&self) {
        let _one = lock(&self.lifecycle);
        lock(&self.state).closed = true;
        self.stop_running();
        self.spoke_wake_up();
        self.spoke_wait_idle();
    }

    /// The deletion failed before anything was removed: sync carries on.
    pub fn resume(&self) {
        lock(&self.state).closed = false;
        self.poke();
    }

    /// Asks the background thread to look again now.
    pub fn poke(&self) {
        *lock(&self.wake.0) = true;
        self.wake.1.notify_all();
        self.spoke_wake_up();
    }

    fn emit(&self, e: SyncEvent) {
        (self.cfg.emit)(e);
    }

    fn store(&self) -> Result<Arc<Store>, String> {
        self.core.store()
    }

    fn enabled(store: &Store) -> bool {
        store
            .get_setting(ENABLED_KEY)
            .ok()
            .flatten()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn require_hub(&self) -> Result<(), String> {
        if self.cfg.role == Role::Hub {
            Ok(())
        } else {
            Err(ERR_NOT_HUB.into())
        }
    }

    // --- the listener's lifetime ---------------------------------------

    fn pairing_open(&self) -> bool {
        lock(&self.state)
            .pairing_until
            .is_some_and(|t| t > Instant::now())
    }

    /// Whether a listener (and the node behind it) should exist now.
    fn want_active(&self) -> bool {
        if self.cfg.role != Role::Hub
            || self.shutdown.load(Ordering::Acquire)
            || lock(&self.state).closed
            || self.core.locked()
        {
            return false;
        }
        if self.pairing_open() {
            return true;
        }
        // A store that can't be opened is the app's to fix (recovery); sync
        // doesn't keep knocking on it.
        if self.core.open_problem().is_some() {
            return false;
        }
        let Ok(store) = self.core.store_even_locked() else {
            return false;
        };
        // An unpair nobody took in time stops holding the listener open.
        if let Err(e) = store.expire_unpair_pending(UNPAIR_WAIT.as_millis() as i64) {
            log::warn!("sync: expiring unpairs: {e}");
        }
        // Any device left (paired, or waiting for a Wipe or an Unpair to be
        // delivered) keeps the listener up.
        Self::enabled(&store) && store.devices().is_ok_and(|d| !d.is_empty())
    }

    /// Makes the listener, the advertisement and the pairing window match
    /// the rule in the module docs. Safe to call from anywhere but under the
    /// store's open (it takes the store).
    pub fn reconcile(self: &Arc<Self>) {
        let _one = lock(&self.lifecycle);
        let want = self.want_active();
        let (have, listening) = {
            let st = lock(&self.state);
            (
                st.running.is_some(),
                st.running
                    .as_ref()
                    .and_then(|r| r.thread.as_ref())
                    .is_some_and(|t| !t.is_finished()),
            )
        };
        match (want, have) {
            (true, false) => self.start_running(),
            (false, true) => self.stop_running(),
            // A network that came back after the listener could not bind,
            // or an accept loop that ended: start over.
            (true, true) if !listening && !(self.cfg.addrs)().is_empty() => {
                self.stop_running();
                self.start_running();
            }
            _ => {}
        }
    }

    fn start_running(self: &Arc<Self>) {
        if let Err(e) = self.try_start() {
            log::warn!("sync did not start: {e}");
        }
    }

    fn try_start(self: &Arc<Self>) -> Result<(), String> {
        let store = self.core.store_even_locked()?;
        let identity = self.identity(&store)?;
        let addrs = (self.cfg.addrs)();
        let preferred = store
            .get_setting(PORT_KEY)
            .ok()
            .flatten()
            .and_then(|v| v.as_u64())
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(0);
        // The port is chosen once and kept; a port taken since falls back.
        let listener = if addrs.is_empty() {
            None
        } else {
            match Listener::bind(&addrs, preferred).or_else(|e| {
                if preferred == 0 {
                    Err(e)
                } else {
                    Listener::bind(&addrs, 0)
                }
            }) {
                Ok(l) => Some(l),
                Err(e) => {
                    log::warn!("sync cannot listen: {e}");
                    None
                }
            }
        };
        let port = listener.as_ref().map_or(preferred, Listener::port);
        if listener.is_some() && port != preferred {
            let _ = store.set_setting(PORT_KEY, &serde_json::json!(port));
        }
        let weak = Arc::downgrade(self);
        let watched: Arc<dyn SyncStore> = Arc::new(Watched {
            store,
            seen: Arc::new(move |s| {
                if let Some(me) = weak.upgrade() {
                    me.on_seen(s);
                }
            }),
        });
        let node = Arc::new(HubNode::new(
            watched,
            identity,
            Arc::new(SystemClock),
            &self.cfg.device_name,
            port,
        ));
        let shared = Arc::new(Mutex::new(
            listener
                .as_ref()
                .map(Listener::local_addrs)
                .unwrap_or_default(),
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = match listener {
            Some(l) => {
                let (me, node, shared, stop) =
                    (self.clone(), node.clone(), shared.clone(), stop.clone());
                Some(
                    std::thread::Builder::new()
                        .name("ghi-sync-accept".into())
                        .spawn(move || me.accept_loop(node, l, shared, stop))
                        .map_err(|e| e.to_string())?,
                )
            }
            None => None,
        };
        lock(&self.state).running = Some(Running {
            node,
            addrs: shared,
            stop,
            thread,
        });
        Ok(())
    }

    fn stop_running(&self) {
        let running = lock(&self.state).running.take();
        if let Some(mut r) = running {
            r.stop.store(true, Ordering::Release);
            if let Some(t) = r.thread.take() {
                let _ = t.join();
            }
            r.node.close_pairing();
        }
        self.end_sessions();
    }

    /// Ends every session and waits (bounded) for the threads to let go of
    /// the store.
    fn end_sessions(&self) {
        for e in lock(&self.sessions).values() {
            e.kill.store(true, Ordering::Release);
        }
        let until = Instant::now() + STOP_WAIT;
        while !lock(&self.sessions).is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// This device's identity, made on first use. A store restored from an
    /// archive has a new device gid: the identity of the old one and the pins
    /// made with it are dropped and a new identity is made.
    fn identity(&self, store: &Store) -> Result<Identity, String> {
        let secrets = self.core.secrets()?;
        let gid = store.sync_device_gid().map_err(|e| e.to_string())?;
        match Identity::load(secrets.as_ref()) {
            Ok(Some(id)) if id.device_gid == gid => return Ok(id),
            Ok(None) => {}
            // Stale or unreadable: nothing paired with it can be used.
            Ok(Some(_)) | Err(_) => drop_restored_pins(store, secrets.as_ref())?,
        }
        Identity::load_or_create(secrets.as_ref(), &gid).map_err(|e| e.to_string())
    }

    fn accept_loop(
        self: Arc<Self>,
        node: Arc<HubNode>,
        mut listener: Listener,
        shared: Arc<Mutex<Vec<SocketAddr>>>,
        stop: Arc<AtomicBool>,
    ) {
        let limits = listener.limits();
        let mut advert = self.advertise(&listener);
        let mut last_rebind = Instant::now();
        while !stop.load(Ordering::Acquire) {
            if last_rebind.elapsed() >= REBIND_EVERY {
                last_rebind = Instant::now();
                match listener.rebind(&(self.cfg.addrs)()) {
                    Ok(true) => {
                        *lock(&shared) = listener.local_addrs();
                        let _ = advert.take();
                        advert = self.advertise(&listener);
                    }
                    Ok(false) => {}
                    Err(e) => log::warn!("sync listener: {e}"),
                }
            }
            match listener.accept_timeout(Duration::from_millis(250)) {
                Ok(Some((stream, peer))) => {
                    self.spawn_session(node.clone(), limits.clone(), stream, peer.ip());
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!("sync listener ended: {e}");
                    break;
                }
            }
        }
        // The advertisement ends with the loop.
        let _ = advert;
    }

    #[cfg(feature = "mdns")]
    fn advertise(&self, listener: &Listener) -> Option<lan::MdnsAdvertiser> {
        if !self.cfg.advertise {
            return None;
        }
        let ips: Vec<IpAddr> = listener.local_addrs().iter().map(SocketAddr::ip).collect();
        match lan::MdnsAdvertiser::start(&ips, listener.port()) {
            Ok(a) => Some(a),
            Err(e) => {
                log::warn!("sync advertisement failed: {e}");
                None
            }
        }
    }

    #[cfg(not(feature = "mdns"))]
    fn advertise(&self, _listener: &Listener) -> Option<()> {
        None
    }

    fn spawn_session(
        self: &Arc<Self>,
        node: Arc<HubNode>,
        limits: Arc<Limits>,
        stream: LanStream,
        ip: IpAddr,
    ) {
        // Locked: no session (the closed connection is all the peer learns).
        if self.core.locked() {
            return;
        }
        let kill = Arc::new(AtomicBool::new(false));
        let id = self.next_session.fetch_add(1, Ordering::Relaxed);
        {
            let mut sessions = lock(&self.sessions);
            if sessions.values().filter(|e| e.ip == ip).count() >= MAX_PER_ADDRESS {
                return;
            }
            sessions.insert(
                id,
                SessionEntry {
                    kill: kill.clone(),
                    ip,
                    device: None,
                },
            );
        }
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("ghi-sync-session".into())
            .spawn(move || {
                let _guard = SessionGuard {
                    svc: me.clone(),
                    id,
                };
                if let Ok(stream) = Killable::new(stream, kill) {
                    let again = me.clone();
                    let _ = me.serve_connection_with(
                        &node,
                        stream,
                        Some(ip),
                        Some(&limits),
                        move |gid| again.replace_older_sessions(id, gid),
                    );
                }
                // The store is let go before the registry says so.
                drop(node);
            });
        if spawned.is_err() {
            lock(&self.sessions).remove(&id);
        }
    }

    /// Serves one accepted connection (pairing or session) and reacts to how
    /// it ended. The accept loop calls this with a socket; tests call it with
    /// a memory pipe.
    pub fn serve_connection<S: ByteStream>(
        self: &Arc<Self>,
        node: &HubNode,
        stream: S,
        ip: Option<IpAddr>,
        limits: Option<&Limits>,
    ) -> ghi_sync::Result<Served> {
        self.serve_connection_with(node, stream, ip, limits, |_| {})
    }

    /// [`SyncService::serve_connection`], telling `identified` the device gid
    /// once a session knows who it serves.
    pub fn serve_connection_with<S: ByteStream>(
        self: &Arc<Self>,
        node: &HubNode,
        stream: S,
        ip: Option<IpAddr>,
        limits: Option<&Limits>,
        identified: impl FnMut(&str) + Send + 'static,
    ) -> ghi_sync::Result<Served> {
        let r = node.serve_with(stream, ip, limits, identified);
        self.after_serve(&r);
        if node.pairing_failed_out() {
            // Three failed pairings: the code is dead. Drop it and tell the
            // sheet, which shows a new one.
            node.close_pairing();
            lock(&self.state).pairing_until = None;
            self.emit(SyncEvent::PairCodeSpent);
        }
        r
    }

    /// One session per device (doc 07 §5.3): session `id` serves `gid`, so
    /// any older session of that device is stale (a phone that went to the
    /// background or changed networks) and ends now.
    fn replace_older_sessions(&self, id: u64, gid: &str) {
        let mut sessions = lock(&self.sessions);
        for (other, e) in sessions.iter() {
            if *other != id && e.device.as_deref() == Some(gid) {
                e.kill.store(true, Ordering::Release);
            }
        }
        if let Some(me) = sessions.get_mut(&id) {
            me.device = Some(gid.to_string());
        }
    }

    // --- what a connection did -----------------------------------------

    fn after_serve(self: &Arc<Self>, r: &ghi_sync::Result<Served>) {
        let mut closed_by = None;
        match r {
            Ok(Served::Paired(p)) => self.on_paired(&p.device_gid),
            Ok(Served::Session(rep)) => {
                closed_by = rep.closed_by;
                self.on_session(rep);
            }
            Err(e) => {
                if let Some(code) = error_code(e) {
                    lock(&self.state).last_error = Some(code);
                    self.emit(SyncEvent::Error { code });
                }
            }
        }
        // Whatever ended, leases the phone opened get their jobs (a crash
        // between the lease and its job is repaired here too).
        self.start_lease_jobs();
        self.refresh_known(closed_by);
        if closed_by.is_some() {
            // A pin is gone: the listener may have nothing left to wait for.
            self.poke();
        }
    }

    fn on_paired(self: &Arc<Self>, gid: &str) {
        {
            let mut st = lock(&self.state);
            st.pairing_until = None;
            st.last_error = None;
        }
        if let Ok(store) = self.core.store_even_locked()
            && let Ok(Some(d)) = store.device(gid)
        {
            self.emit(SyncEvent::Paired {
                device: device_row(&d),
            });
        }
    }

    fn on_session(self: &Arc<Self>, rep: &SessionReport) {
        lock(&self.state).last_error = None;
        if rep.needs_confirm > 0
            && let Some(peer) = &rep.peer
        {
            self.ask_mass_delete(peer, rep.needs_confirm);
        }
        if let Ok(store) = self.core.store_even_locked() {
            self.emit(SyncEvent::Progress {
                pending: pending_changes(&store),
            });
        }
    }

    fn ask_mass_delete(&self, peer: &str, count: usize) {
        let name = {
            let mut st = lock(&self.state);
            if st.declined.get(peer).is_some_and(|t| *t > Instant::now()) {
                return;
            }
            st.pending_confirm = Some(peer.to_string());
            st.known
                .as_ref()
                .and_then(|k| k.get(peer))
                .map(|(n, _)| n.clone())
        };
        self.emit(SyncEvent::NeedsConfirm {
            device: name.unwrap_or_else(|| "device".to_string()),
            count: u32::try_from(count).unwrap_or(u32::MAX),
        });
    }

    /// Compares the paired devices with the last look: one that is gone was
    /// unpaired by the peer, or wiped.
    fn refresh_known(&self, closed_by: Option<ControlOutcome>) {
        if self.core.open_problem().is_some() {
            return;
        }
        let Ok(store) = self.core.store_even_locked() else {
            return;
        };
        let Ok(devices) = store.devices() else {
            return;
        };
        let now: HashMap<String, (String, StoreDeviceState)> = devices
            .iter()
            .map(|d| (d.gid.clone(), (d.name.clone(), d.state)))
            .collect();
        let old = lock(&self.state).known.replace(now.clone());
        let Some(old) = old else { return };
        for (gid, (name, state)) in old {
            if now.contains_key(&gid) {
                continue;
            }
            if state == StoreDeviceState::WipePending {
                self.wipe_finished();
                self.emit(SyncEvent::WipeDone { gid });
            } else if state == StoreDeviceState::UnpairPending {
                // The user's own unpair, now delivered (or given up on).
                self.emit(SyncEvent::Unpaired {
                    gid,
                    name,
                    by_peer: false,
                });
            } else if closed_by == Some(ControlOutcome::Wiped) {
                self.emit(SyncEvent::Unpaired {
                    gid: gid.clone(),
                    name,
                    by_peer: true,
                });
                self.wipe_finished();
                self.emit(SyncEvent::WipeDone { gid });
            } else {
                self.emit(SyncEvent::Unpaired {
                    gid,
                    name,
                    by_peer: true,
                });
            }
        }
    }

    /// The phone's "Unpair and delete on <computer>" is over.
    fn wipe_finished(&self) {
        let mut st = lock(&self.state);
        if self.cfg.role == Role::Spoke && st.delete.state == Some(DeleteEverywhereState::Waiting) {
            st.delete = DeleteState {
                state: Some(DeleteEverywhereState::Done),
                waiting_for: Vec::new(),
            };
        }
    }

    fn on_seen(&self, seen: Seen) {
        match seen {
            Seen::Conflict(meeting) => {
                let has = self
                    .core
                    .store_even_locked()
                    .ok()
                    .and_then(|s| s.conflict_copies(&meeting).ok())
                    .is_some_and(|c| !c.is_empty());
                if has {
                    self.emit(SyncEvent::Conflict { meeting });
                }
            }
            Seen::Settings => {
                // The settings the app caches changed under it.
                *lock(self.core.settings_cache()) = None;
                (self.cfg.settings_changed)();
            }
            Seen::Lease => self.start_lease_jobs(),
        }
    }

    /// Turns every lease a phone granted into its local job: the final pass
    /// (which queues the notes under the same lease), or the final notes when
    /// that is all that was leased. One job per lease, ever.
    fn start_lease_jobs(&self) {
        let Ok(store) = self.core.store_even_locked() else {
            return;
        };
        let Ok(open) = store.leases_open() else {
            return;
        };
        let mut queued = false;
        for l in open
            .iter()
            .filter(|l| l.role == LeaseRole::Holder && l.state == "granted")
        {
            let has = |k: &str| l.kinds.iter().any(|x| x == k);
            let kind = if has(FINAL_PASS_JOB) {
                FINAL_PASS_JOB
            } else if has(NOTES_FINAL_JOB) {
                NOTES_FINAL_JOB
            } else {
                continue;
            };
            let Ok(jobs) = store.jobs_for_meeting(&l.meeting_gid) else {
                continue;
            };
            let exists = jobs.iter().any(|j| {
                j.kind == kind
                    && j.payload.get("lease").and_then(serde_json::Value::as_str)
                        == Some(l.job_uuid.as_str())
            });
            if exists {
                continue;
            }
            match store.enqueue_job(
                Some(&l.meeting_gid),
                kind,
                JOB_PAYLOAD_VERSION,
                &serde_json::json!({"lease": l.job_uuid, "epoch": l.epoch}),
            ) {
                Ok(_) => queued = true,
                Err(e) => log::warn!("leased job not queued: {e}"),
            }
        }
        if queued {
            self.core.notify_jobs();
        }
    }

    // --- commands --------------------------------------------------------

    pub fn status(&self) -> Result<SyncStatus, String> {
        let store = self.store()?;
        let devices = store.devices().map_err(|e| e.to_string())?;
        let enabled = Self::enabled(&store);
        // Waiting to go to a device that has not been here for a while.
        let stale = |d: &Device| {
            d.last_seen.is_none_or(|t| {
                now_ms() - t
                    > i64::try_from(self.cfg.reachable_within.as_millis()).unwrap_or(i64::MAX)
            })
        };
        let pending_on_phone = match self.cfg.role {
            Role::Hub if enabled && devices.iter().any(stale) => pending_changes(&store),
            Role::Spoke if enabled && !devices.is_empty() => pending_changes(&store),
            _ => 0,
        };
        Ok(SyncStatus {
            enabled,
            paired: devices.iter().map(device_row).collect(),
            pending_on_phone,
            local_only: true,
            last_error_code: lock(&self.state).last_error.map(code_text),
        })
    }

    pub fn set_enabled(self: &Arc<Self>, enabled: bool) -> Result<SyncStatus, String> {
        let store = self.store()?;
        if enabled {
            // The identity is made now, so a failure shows here.
            self.identity(&store)?;
        } else {
            self.close_pairing();
        }
        store
            .set_setting(ENABLED_KEY, &serde_json::json!(enabled))
            .map_err(|e| e.to_string())?;
        self.reconcile();
        self.status()
    }

    /// Opens the pairing window and returns the QR code to show. The code is
    /// a secret: it is rendered and dropped, never logged or kept.
    pub fn pair_open(self: &Arc<Self>) -> Result<PairOffer, String> {
        self.require_hub()?;
        let store = self.store()?;
        if !Self::enabled(&store) {
            return Err(ERR_OFF.into());
        }
        lock(&self.state).pairing_until = Some(Instant::now() + PAIR_TTL);
        self.reconcile();
        let (node, addrs) = {
            let st = lock(&self.state);
            st.running
                .as_ref()
                .map(|r| (r.node.clone(), lock(&r.addrs).clone()))
                .ok_or_else(|| ERR_NO_NETWORK.to_string())?
        };
        let text = if addrs.is_empty() {
            None
        } else {
            node.open_pairing_code(&addrs).ok().map(Zeroizing::new)
        };
        let Some(text) = text else {
            self.close_pairing();
            return Err(ERR_NO_NETWORK.into());
        };
        let qr_svg = qr::svg(&text).map_err(|e| e.to_string())?;
        Ok(PairOffer {
            qr_svg,
            expires_ms: PAIR_CODE_TTL_MS,
        })
    }

    pub fn pair_close(self: &Arc<Self>) -> Result<(), String> {
        self.close_pairing();
        Ok(())
    }

    fn close_pairing(self: &Arc<Self>) {
        lock(&self.state).pairing_until = None;
        let node = lock(&self.state).running.as_ref().map(|r| r.node.clone());
        if let Some(n) = node {
            n.close_pairing();
        }
        self.reconcile();
    }

    pub fn devices(&self) -> Result<Vec<DeviceRow>, String> {
        let store = self.store()?;
        Ok(store
            .devices()
            .map_err(|e| e.to_string())?
            .iter()
            .map(device_row)
            .collect())
    }

    fn device_of(store: &Store, gid: &str) -> Result<Device, String> {
        store
            .device(gid)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| ERR_UNKNOWN_DEVICE.to_string())
    }

    /// Forgets a device; what was synced stays on both.
    pub fn unpair(self: &Arc<Self>, gid: &str) -> Result<(), String> {
        let store = self.store()?;
        let d = Self::device_of(&store, gid)?;
        if self.cfg.role == Role::Spoke {
            // The computer forgets this phone too, when it can be reached.
            self.spoke_tell_hub(&store, wire::Control::Unpair);
        } else {
            // The phone is told when it next connects (or on its next ping);
            // the listener stays up until then.
            store.set_unpair_pending(gid).map_err(|e| e.to_string())?;
            if let Some(e) = lock(&self.state)
                .known
                .as_mut()
                .and_then(|k| k.get_mut(gid))
            {
                e.1 = StoreDeviceState::UnpairPending;
            }
            self.reconcile();
            return Ok(());
        }
        store.unpin_device(gid).map_err(|e| e.to_string())?;
        if let Some(k) = lock(&self.state).known.as_mut() {
            k.remove(gid);
        }
        self.emit(SyncEvent::Unpaired {
            gid: d.gid,
            name: d.name,
            by_peer: false,
        });
        self.reconcile();
        Ok(())
    }

    /// Forgets a device and has it delete what it got from this one: its next
    /// session delivers `Wipe`.
    pub fn unpair_and_wipe(self: &Arc<Self>, gid: &str) -> Result<(), String> {
        let store = self.store()?;
        Self::device_of(&store, gid)?;
        store.set_wipe_pending(gid).map_err(|e| e.to_string())?;
        if let Some(k) = lock(&self.state).known.as_mut()
            && let Some(e) = k.get_mut(gid)
        {
            e.1 = StoreDeviceState::WipePending;
        }
        if self.cfg.role == Role::Spoke {
            // The phone's session delivers it; the status shows the wait.
            let name = Self::device_of(&store, gid)?.name;
            lock(&self.state).delete = DeleteState {
                state: Some(DeleteEverywhereState::Waiting),
                waiting_for: vec![name],
            };
            self.spoke_wake_up();
        }
        self.reconcile();
        Ok(())
    }

    /// The hub never connects out: "sync now" refreshes what it knows (and
    /// the pending count), and the phone's next session does the rest.
    pub fn now(self: &Arc<Self>) -> Result<(), String> {
        let store = self.store()?;
        if !Self::enabled(&store) {
            return Err(ERR_OFF.into());
        }
        self.reconcile();
        self.start_lease_jobs();
        if self.cfg.role == Role::Spoke {
            self.spoke_run_now();
        }
        self.emit(SyncEvent::Progress {
            pending: pending_changes(&store),
        });
        Ok(())
    }

    pub fn conflicts(&self, meeting: &str) -> Result<Vec<ConflictCopy>, String> {
        let store = self.store()?;
        let copies = store.conflict_copies(meeting).map_err(|e| e.to_string())?;
        let names: HashMap<String, String> = store
            .devices()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|d| (d.gid, d.name))
            .collect();
        let own = store.sync_device_gid().map_err(|e| e.to_string())?;
        Ok(copies
            .into_iter()
            .map(|c| ConflictCopy {
                device: if c.origin == own {
                    self.cfg.device_name.clone()
                } else {
                    names
                        .get(&c.origin)
                        .cloned()
                        .unwrap_or_else(|| "another device".to_string())
                },
                target_kind: match c.target_kind.as_str() {
                    "meeting" => ConflictTarget::Title,
                    "segment" => ConflictTarget::Segment,
                    "note" => ConflictTarget::NoteBlock,
                    "action_item" => ConflictTarget::ActionItem,
                    _ => ConflictTarget::Speaker,
                },
                field: c.field.trim_end_matches("_ct").to_string(),
                gid: c.gid,
                text: c.text,
            })
            .collect())
    }

    pub fn conflict_resolve(&self, gid: &str, use_it: bool) -> Result<(), String> {
        match self.store()?.resolve_conflict(gid, use_it) {
            Ok(()) => Ok(()),
            Err(ghi_store::StoreError::NotFound { .. }) => Err(ERR_UNKNOWN_CONFLICT.into()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Answers a [`SyncEvent::NeedsConfirm`]. Accepting lets the device's next
    /// session apply its deletes; refusing keeps everything and stops asking
    /// for a while.
    pub fn confirm_mass_delete(&self, accept: bool) -> Result<(), String> {
        let peer = {
            let mut st = lock(&self.state);
            let Some(peer) = st.pending_confirm.take() else {
                return Err(ERR_NOTHING_TO_CONFIRM.into());
            };
            if !accept {
                st.declined
                    .insert(peer.clone(), Instant::now() + DECLINE_QUIET);
            }
            peer
        };
        if accept {
            self.store()?
                .set_mass_delete_confirmed(&peer, true)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn delete_everywhere_status(&self) -> DeleteEverywhereStatus {
        let st = lock(&self.state);
        DeleteEverywhereStatus {
            state: st.delete.state.unwrap_or(DeleteEverywhereState::Idle),
            waiting_for: st.delete.waiting_for.clone(),
        }
    }

    /// "Delete here only": the wait for paired devices ends now.
    pub fn delete_everywhere_skip(&self) {
        self.skip_wipe.store(true, Ordering::Release);
    }

    /// The first half of "Delete everything" (doc 07 §3.5, §7.9): with
    /// `everywhere`, every paired device is queued for `Wipe`, and this waits
    /// until the devices seen a moment ago have taken it, the wait is
    /// skipped ([`SyncService::delete_everywhere_skip`]) or [`WIPE_WAIT`] is
    /// over. A device nobody has seen lately is not waited for (it keeps its
    /// copies and finds itself unpaired when it comes back). The caller then
    /// deletes the data, which also destroys the identity.
    pub fn delete_everywhere_prepare(self: &Arc<Self>, everywhere: bool) -> Result<(), String> {
        self.skip_wipe.store(false, Ordering::Release);
        lock(&self.queued_wipes).clear();
        lock(&self.state).delete = DeleteState::default();
        let result = self.wait_for_wipes(everywhere);
        // A failed wait leaves the status where the UI can see it end.
        let mut st = lock(&self.state);
        if result.is_err() || st.delete.state == Some(DeleteEverywhereState::Waiting) {
            st.delete = DeleteState {
                state: Some(DeleteEverywhereState::Done),
                waiting_for: Vec::new(),
            };
        }
        result
    }

    /// The local delete failed after [`SyncService::delete_everywhere_prepare`]
    /// queued wipes: those devices are paired again (nothing was removed).
    /// Devices that took their wipe meanwhile are gone and stay gone.
    pub fn rollback_wipes(&self) {
        let queued = std::mem::take(&mut *lock(&self.queued_wipes));
        if queued.is_empty() {
            return;
        }
        let Ok(store) = self.core.store_even_locked() else {
            log::warn!("devices stay queued for a wipe: the store is not open");
            return;
        };
        for gid in &queued {
            if let Err(e) = store.clear_wipe_pending(gid) {
                log::warn!("a device stays queued for a wipe: {e}");
            }
        }
        let mut st = lock(&self.state);
        st.known = None;
        st.delete = DeleteState::default();
    }

    fn wait_for_wipes(self: &Arc<Self>, everywhere: bool) -> Result<(), String> {
        if !everywhere {
            return Ok(());
        }
        let store = self.store()?;
        let devices = store.devices().map_err(|e| e.to_string())?;
        if devices.is_empty() {
            return Ok(());
        }
        let window = i64::try_from(self.cfg.reachable_within.as_millis()).unwrap_or(i64::MAX);
        let near: HashSet<String> = devices
            .iter()
            .filter(|d| d.last_seen.is_some_and(|t| now_ms() - t <= window))
            .map(|d| d.gid.clone())
            .collect();
        for d in &devices {
            let was = d.state == StoreDeviceState::WipePending;
            store.set_wipe_pending(&d.gid).map_err(|e| e.to_string())?;
            if !was {
                lock(&self.queued_wipes).push(d.gid.clone());
            }
        }
        // The devices that can still take it, with the listener up (hub) or
        // the session loop awake (phone).
        self.reconcile();
        self.spoke_wake_up();
        let until = Instant::now() + WIPE_WAIT;
        loop {
            let present = store.devices().map_err(|e| e.to_string())?;
            let left: Vec<String> = present
                .iter()
                .filter(|d| near.contains(&d.gid))
                .map(|d| d.name.clone())
                .collect();
            let listening = self.cfg.role == Role::Spoke || lock(&self.state).running.is_some();
            let over = left.is_empty()
                || !listening
                || self.skip_wipe.load(Ordering::Acquire)
                || Instant::now() >= until;
            let mut st = lock(&self.state);
            if over {
                // Whoever still holds a pin was not reached.
                st.delete = DeleteState {
                    state: Some(DeleteEverywhereState::Done),
                    waiting_for: present.into_iter().map(|d| d.name).collect(),
                };
                return Ok(());
            }
            st.delete = DeleteState {
                state: Some(DeleteEverywhereState::Waiting),
                waiting_for: left,
            };
            drop(st);
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    // --- the background thread -------------------------------------------

    fn tick(self: &Arc<Self>) {
        // An expired pairing window closes with its listener.
        let expired = {
            let mut st = lock(&self.state);
            match st.pairing_until {
                Some(t) if t <= Instant::now() => {
                    st.pairing_until = None;
                    true
                }
                _ => false,
            }
        };
        if expired {
            let node = lock(&self.state).running.as_ref().map(|r| r.node.clone());
            if let Some(n) = node {
                n.close_pairing();
            }
        }
        self.reconcile();
        self.refresh_known(None);
    }

    /// Whether the listener has sockets open right now.
    pub fn listening(&self) -> bool {
        lock(&self.state)
            .running
            .as_ref()
            .and_then(|r| r.thread.as_ref())
            .is_some_and(|t| !t.is_finished())
    }

    /// The live hub node (tests drive connections through it).
    #[cfg(test)]
    pub(crate) fn node(&self) -> Option<Arc<HubNode>> {
        lock(&self.state).running.as_ref().map(|r| r.node.clone())
    }
}

fn tick_loop(weak: Weak<SyncService>) {
    loop {
        let Some(me) = weak.upgrade() else { return };
        if me.shutdown.load(Ordering::Acquire) {
            return;
        }
        me.tick();
        let (flag, cv) = &me.wake;
        let mut woken = lock(flag);
        if !*woken {
            woken = cv
                .wait_timeout(woken, REBIND_EVERY)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        *woken = false;
    }
}

/// Changes still to reach every paired device: tombstones and rows after the
/// slowest device's acknowledged position.
fn pending_changes(store: &Store) -> u32 {
    let Ok(devices) = store.devices() else {
        return 0;
    };
    let Some(since) = devices.iter().map(|d| d.push_seq).min() else {
        return 0;
    };
    let tombs = store.tombs_since(since, 256).map_or(0, |b| b.tombs.len());
    let rows = store
        .changes_since(since, 256)
        .map_or(0, |b| b.changes.len());
    u32::try_from(tombs + rows).unwrap_or(u32::MAX)
}

#[path = "sync_spoke.rs"]
pub mod spoke;

#[cfg(test)]
#[path = "sync_service_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sync_spoke_tests.rs"]
mod spoke_tests;
