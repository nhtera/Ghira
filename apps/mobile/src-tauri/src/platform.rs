// SPDX-License-Identifier: Apache-2.0
//! The C ABI between Rust and the Swift side (`native/ios/GhiAudio`, header
//! `include/ghi_ios.h`).
//!
//! - Rust → Swift (`ghi_swift_*`, `@_cdecl` in Swift): start/stop the audio
//!   session, drive the Live Activity, read device stats.
//! - Swift → Rust (`ghi_ios_*`, exported here): PCM blocks from the tap,
//!   app lifecycle (resign/become active), thermal state, interruptions, and
//!   the Live Activity's Stop/Mark intents.
//!
//! On other targets (host tests, desktop preview) the Swift calls are no-ops.

use crate::session::{self, Phase};

/// Device numbers for the metrics log; `None` when unknown.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceStats {
    pub thermal: Option<i32>,
    pub memory_mb: Option<f64>,
    pub battery: Option<f64>,
}

/// The Live Activity's phase on the Swift side: `ActivityPhase` in
/// `native/ios/Shared/ActivityPhase.swift`, case for case. The discriminants
/// are the C ABI contract (a test pins them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
enum ActivityPhase {
    Loading = 0,
    Live = 1,
    Locked = 2,
    CatchingUp = 3,
    Hot = 4,
    Interrupted = 5,
    Finishing = 6,
    Done = 7,
    RecordOnly = 8,
    /// Interrupted by something other than a phone call, or paused by the user.
    Paused = 9,
}

impl From<Phase> for ActivityPhase {
    fn from(p: Phase) -> ActivityPhase {
        match p {
            Phase::Loading => ActivityPhase::Loading,
            Phase::Live => ActivityPhase::Live,
            Phase::Locked => ActivityPhase::Locked,
            Phase::CatchingUp => ActivityPhase::CatchingUp,
            Phase::Hot => ActivityPhase::Hot,
            Phase::Interrupted => ActivityPhase::Interrupted,
            Phase::Finishing => ActivityPhase::Finishing,
            Phase::Done => ActivityPhase::Done,
            Phase::RecordOnly => ActivityPhase::RecordOnly,
        }
    }
}

#[cfg(target_os = "ios")]
mod swift {
    unsafe extern "C" {
        /// Installs the lifecycle, thermal and interruption observers.
        pub fn ghi_swift_init();
        /// Configures and starts the audio session and tap; 0 on success,
        /// else an error code (1 permission denied, 2 session, 3 engine).
        pub fn ghi_swift_audio_start() -> i32;
        pub fn ghi_swift_audio_stop();
        pub fn ghi_swift_activity_start(phase: i32);
        pub fn ghi_swift_activity_update(phase: i32, marks: u32);
        pub fn ghi_swift_activity_end();
        /// `ProcessInfo.thermalState` raw value.
        pub fn ghi_swift_thermal_state() -> i32;
        /// Physical footprint in bytes.
        pub fn ghi_swift_memory_footprint() -> u64;
        /// 0..1, or < 0 when unknown.
        pub fn ghi_swift_battery_level() -> f32;
        /// Sets `NSURLIsExcludedFromBackupKey` on a directory; false on failure.
        pub fn ghi_swift_exclude_from_backup(path: *const std::ffi::c_char) -> bool;
        /// A phone call is active (CXCallObserver).
        pub fn ghi_swift_call_active() -> bool;
        /// The process was launched in the background.
        pub fn ghi_swift_launched_in_background() -> bool;
        /// The network path is expensive or constrained (NWPathMonitor).
        pub fn ghi_swift_on_expensive_network() -> bool;
        /// Writes the machine identifier (`iPhone16,1`) as a NUL-terminated
        /// string into `buf` (capacity `cap`); returns its length.
        pub fn ghi_swift_device_model(buf: *mut std::ffi::c_char, cap: usize) -> usize;
        /// `ProcessInfo.physicalMemory` in bytes.
        pub fn ghi_swift_physical_memory() -> u64;
        /// Dynamic Type as a multiplier (1.0 = Large), capped at 2.0.
        pub fn ghi_swift_text_scale() -> f32;
        /// Presents the share sheet for a file; false if it could not be shown.
        pub fn ghi_swift_share_file(path: *const std::ffi::c_char) -> bool;
        /// Opens this app's page in Settings.
        pub fn ghi_swift_open_settings();
        /// The main WKWebView stops adding the safe-area insets to the page's layout.
        pub fn ghi_swift_webview_never_adjust(webview: *mut std::ffi::c_void);
        /// `beginBackgroundTask`; returns a token for `end_bg_task` (0: none granted).
        pub fn ghi_swift_begin_bg_task(name: *const std::ffi::c_char) -> u64;
        pub fn ghi_swift_end_bg_task(token: u64);
        /// Covers the window (app switcher snapshot, app lock).
        pub fn ghi_swift_set_privacy_cover(on: bool);
        /// Nanoseconds from a clock that keeps counting while the device sleeps
        /// (`mach_continuous_time`).
        pub fn ghi_swift_continuous_ns() -> u64;
        /// 0 not determined, 1 granted, 2 denied.
        pub fn ghi_swift_mic_permission() -> i32;
        /// Shows the system prompt (first time only); poll `mic_permission`.
        pub fn ghi_swift_request_mic_permission();
        /// 0 not determined, 1 full access, 2 denied (or restricted, or write-only).
        pub fn ghi_swift_calendar_access() -> i32;
        /// Shows the system prompt (first time only); poll `calendar_access`.
        pub fn ghi_swift_request_calendar_access();
        /// The events starting in `[from_ms, to_ms)` as a JSON array in a heap
        /// string (free it with `ghi_swift_string_free`); NULL without access.
        pub fn ghi_swift_calendar_events(from_ms: i64, to_ms: i64) -> *mut std::ffi::c_char;
        pub fn ghi_swift_string_free(s: *mut std::ffi::c_char);
        /// `volumeAvailableCapacityForImportantUsage` of the app's volume in
        /// bytes (counts purgeable space); < 0 when unknown.
        pub fn ghi_swift_available_capacity() -> i64;
        pub fn ghi_swift_qr_scan_start();
        pub fn ghi_swift_qr_scan_stop();
        pub fn ghi_swift_browse_start();
        pub fn ghi_swift_browse_stop();
    }
}

/// Keeps `dir` (recordings, models, logs) out of iCloud/Finder device
/// backups: meeting audio must not leave the device without an opt-in.
pub fn exclude_from_backup(dir: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
        if !unsafe { swift::ghi_swift_exclude_from_backup(c.as_ptr()) } {
            return Err(format!("could not exclude {} from backups", dir.display()));
        }
    }
    let _ = dir;
    Ok(())
}

/// Right after the main webview is built: it fills the screen (the page pads
/// for the safe areas itself). `webview` is Tauri's platform webview pointer
/// (the `WKWebView`); the call runs on the main thread.
#[cfg(target_os = "ios")]
pub fn webview_fill_screen(webview: *mut std::ffi::c_void) {
    // SAFETY: Swift only takes an unretained reference to the live WKWebView.
    unsafe { swift::ghi_swift_webview_never_adjust(webview) }
}

/// Called once at startup.
pub fn init() {
    // SAFETY: plain C call into Swift.
    #[cfg(target_os = "ios")]
    unsafe {
        swift::ghi_swift_init()
    }
}

#[cfg(target_os = "ios")]
pub fn audio_start() -> Result<(), String> {
    // SAFETY: plain C call into Swift; no arguments.
    match unsafe { swift::ghi_swift_audio_start() } {
        0 => Ok(()),
        1 => Err("microphone access is off: allow it in Settings → Ghira".into()),
        // Resume while a call is still on: the UI keeps asking until it ends.
        4 => Err(crate::session::ERR_CALL_ACTIVE.into()),
        code => Err(format!("the audio session could not start (code {code})")),
    }
}

#[cfg(not(target_os = "ios"))]
pub fn audio_start() -> Result<(), String> {
    Ok(())
}

pub fn audio_stop() {
    // SAFETY: plain C call into Swift.
    #[cfg(target_os = "ios")]
    unsafe {
        swift::ghi_swift_audio_stop()
    }
}

/// The Live Activity's code: an interruption is a "call" only while a call is on.
fn activity_code(phase: Phase) -> i32 {
    match ActivityPhase::from(phase) {
        ActivityPhase::Interrupted if !call_active() => ActivityPhase::Paused as i32,
        other => other as i32,
    }
}

pub fn activity_start(phase: Phase) {
    let _code = activity_code(phase);
    // SAFETY: plain C call into Swift.
    #[cfg(target_os = "ios")]
    unsafe {
        swift::ghi_swift_activity_start(_code)
    }
}

/// Updates the Live Activity, or ends it once the session is `finished`
/// (capture stopped and the engine done).
pub fn activity_update(phase: Phase, marks: u32, finished: bool) {
    let _args = (activity_code(phase), marks, finished);
    // SAFETY: plain C calls into Swift.
    #[cfg(target_os = "ios")]
    unsafe {
        if finished {
            swift::ghi_swift_activity_end()
        } else {
            swift::ghi_swift_activity_update(_args.0, _args.1)
        }
    }
}

#[cfg(target_os = "ios")]
pub fn device_stats() -> DeviceStats {
    // SAFETY: plain C calls into Swift.
    let (thermal, footprint, battery) = unsafe {
        (
            swift::ghi_swift_thermal_state(),
            swift::ghi_swift_memory_footprint(),
            swift::ghi_swift_battery_level(),
        )
    };
    DeviceStats {
        thermal: Some(thermal),
        memory_mb: Some(footprint as f64 / (1024.0 * 1024.0)),
        battery: (battery >= 0.0).then_some(battery as f64),
    }
}

#[cfg(not(target_os = "ios"))]
pub fn device_stats() -> DeviceStats {
    DeviceStats::default()
}

// --- Swift → Rust -----------------------------------------------------------

/// One block of mono PCM from the audio tap, at `rate` Hz; `host_ns` is the
/// block's host time in nanoseconds.
///
/// # Safety
/// `samples` points to `len` readable floats (or `len` is 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghi_ios_push_pcm(
    samples: *const f32,
    len: usize,
    rate: f64,
    host_ns: u64,
) {
    if samples.is_null() || len == 0 {
        return;
    }
    // SAFETY: the caller guarantees `len` floats at `samples`.
    let pcm = unsafe { std::slice::from_raw_parts(samples, len) };
    session::push_pcm(pcm, rate, host_ns);
}

/// The app will resign active: no new engine steps (no new GPU work).
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_suspend() {
    crate::lifecycle::resign_active();
}

/// The app entered the background (main thread, never blocks). Returns
/// `false` if an engine step overlapped the transition: the engine drops its
/// results, reloads the models and redoes that audio. Also tells the job
/// runner (`JobRunner::app_inactive`).
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_entered_background() -> bool {
    crate::lifecycle::entered_background()
}

/// The app is active again (`didBecomeActive`): the engine may use the GPU and
/// catches up; jobs may run.
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_resume() {
    crate::lifecycle::become_active();
}

/// `ProcessInfo.thermalState` changed (0 nominal … 3 critical).
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_thermal_changed(state: i32) {
    crate::lifecycle::thermal_changed(state);
}

/// An audio interruption began (`true`) or ended (`false`).
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_interruption(began: bool) {
    crate::lifecycle::interruption(began);
}

/// The Live Activity's Stop intent.
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_stop_requested() {
    crate::lifecycle::stop_requested();
}

/// The Live Activity's Mark intent.
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_mark_requested() {
    crate::lifecycle::mark_requested();
}

/// A phone call started or ended (`CXCallObserver`).
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_call_active_changed(active: bool) {
    crate::lifecycle::call_active_changed(active);
}

// --- Rust → Swift wrappers (no-ops off iOS) ---------------------------------

/// Wrappers that later slices wire up (16-D session, 16-E native, 16-G services).
#[allow(dead_code)]
mod wrappers {
    use crate::cmd::calendar::CalendarAccess;
    use crate::cmd::onboarding::MicPermission;

    #[cfg(target_os = "ios")]
    use super::swift;

    pub fn call_active() -> bool {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        return unsafe { swift::ghi_swift_call_active() };
        #[cfg(not(target_os = "ios"))]
        false
    }

    /// The process was launched in the background (a relaunch, an intent):
    /// the job runner starts paused.
    pub fn launched_in_background() -> bool {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        return unsafe { swift::ghi_swift_launched_in_background() };
        #[cfg(not(target_os = "ios"))]
        false
    }

    /// Cellular, Personal Hotspot or Low Data Mode (`NWPath.isExpensive` /
    /// `isConstrained`); never off iOS.
    pub fn on_expensive_network() -> bool {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        return unsafe { swift::ghi_swift_on_expensive_network() };
        #[cfg(not(target_os = "ios"))]
        false
    }

    /// The machine identifier; on the simulator the simulated model.
    pub fn device_model() -> String {
        #[cfg(target_os = "ios")]
        {
            let mut buf = [0 as std::ffi::c_char; 64];
            // SAFETY: `buf` is writable for `buf.len()` bytes; Swift NUL-terminates.
            let n = unsafe { swift::ghi_swift_device_model(buf.as_mut_ptr(), buf.len()) };
            let bytes: Vec<u8> = buf
                .iter()
                .take(n.min(buf.len() - 1))
                .map(|&c| c as u8)
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }
        #[cfg(not(target_os = "ios"))]
        String::new()
    }

    pub fn physical_memory() -> u64 {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        return unsafe { swift::ghi_swift_physical_memory() };
        #[cfg(not(target_os = "ios"))]
        0
    }

    pub fn text_scale() -> f32 {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        return unsafe { swift::ghi_swift_text_scale() };
        #[cfg(not(target_os = "ios"))]
        1.0
    }

    pub fn share_file(path: &std::path::Path) -> Result<(), String> {
        #[cfg(target_os = "ios")]
        {
            use std::os::unix::ffi::OsStrExt;
            let c =
                std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
            // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
            if unsafe { swift::ghi_swift_share_file(c.as_ptr()) } {
                return Ok(());
            }
        }
        let _ = path;
        Err("the share sheet could not be shown".into())
    }

    pub fn open_settings() {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        unsafe {
            swift::ghi_swift_open_settings()
        }
    }

    /// Asks iOS for background time (finishing a stop, an export); 0 when none was granted.
    pub fn begin_bg_task(name: &str) -> u64 {
        #[cfg(target_os = "ios")]
        if let Ok(c) = std::ffi::CString::new(name) {
            // SAFETY: `c` is a valid NUL-terminated string for the duration of the call.
            return unsafe { swift::ghi_swift_begin_bg_task(c.as_ptr()) };
        }
        let _ = name;
        0
    }

    pub fn end_bg_task(token: u64) {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        unsafe {
            swift::ghi_swift_end_bg_task(token)
        }
        let _ = token;
    }

    pub fn set_privacy_cover(on: bool) {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        unsafe {
            swift::ghi_swift_set_privacy_cover(on)
        }
        let _ = on;
    }

    /// A clock that includes device sleep (phase 15 needs it for lease times).
    pub fn continuous_ns() -> u64 {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        return unsafe { swift::ghi_swift_continuous_ns() };
        #[cfg(not(target_os = "ios"))]
        0
    }

    pub fn mic_permission() -> MicPermission {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        return match unsafe { swift::ghi_swift_mic_permission() } {
            1 => MicPermission::Granted,
            2 => MicPermission::Denied,
            _ => MicPermission::NotDetermined,
        };
        #[cfg(not(target_os = "ios"))]
        MicPermission::Granted
    }

    /// Free space for important usage, counting what iOS can purge; `None`
    /// off iOS or when Swift cannot tell (callers fall back to `statvfs`).
    pub fn available_capacity() -> Option<u64> {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        return u64::try_from(unsafe { swift::ghi_swift_available_capacity() }).ok();
        #[cfg(not(target_os = "ios"))]
        None
    }

    /// Calendar access (EventKit); never prompts. `Unavailable` off iOS.
    pub fn calendar_access() -> CalendarAccess {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        return match unsafe { swift::ghi_swift_calendar_access() } {
            0 => CalendarAccess::NotDetermined,
            1 => CalendarAccess::Authorized,
            _ => CalendarAccess::Denied,
        };
        #[cfg(not(target_os = "ios"))]
        CalendarAccess::Unavailable
    }

    /// Shows the calendar prompt (only the first time); poll `calendar_access`.
    pub fn request_calendar_access() {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        unsafe {
            swift::ghi_swift_request_calendar_access()
        }
    }

    /// The raw events starting in `[from_ms, to_ms)` as JSON; `None` without
    /// access (and off iOS).
    pub fn calendar_events_json(from_ms: i64, to_ms: i64) -> Option<String> {
        #[cfg(target_os = "ios")]
        {
            // SAFETY: Swift returns NULL or a NUL-terminated heap string that
            // is read once and handed back to Swift to free.
            unsafe {
                let p = swift::ghi_swift_calendar_events(from_ms, to_ms);
                if p.is_null() {
                    return None;
                }
                let s = std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned();
                swift::ghi_swift_string_free(p);
                Some(s)
            }
        }
        #[cfg(not(target_os = "ios"))]
        {
            let _ = (from_ms, to_ms);
            None
        }
    }

    /// Starts the camera scan for the desktop's pairing code (phase 15).
    pub fn qr_scan_start() {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        unsafe {
            swift::ghi_swift_qr_scan_start()
        }
    }

    pub fn qr_scan_stop() {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        unsafe {
            swift::ghi_swift_qr_scan_stop()
        }
    }

    /// Starts browsing for the desktop's sync service (phase 15).
    pub fn browse_start() {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        unsafe {
            swift::ghi_swift_browse_start()
        }
    }

    pub fn browse_stop() {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        unsafe {
            swift::ghi_swift_browse_stop()
        }
    }

    pub fn request_mic_permission() {
        // SAFETY: plain C call into Swift.
        #[cfg(target_os = "ios")]
        unsafe {
            swift::ghi_swift_request_mic_permission()
        }
    }
}

pub use wrappers::*;

// --- More Swift → Rust ---------------------------------------------------------

/// Dynamic Type changed: `scale` multiplies the root font size (capped at 2.0).
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_text_scale_changed(scale: f32) {
    crate::cmd::events::emit(crate::cmd::MobileEvent::TextScale { scale });
}

/// The audio route changed (`AVAudioSession.RouteChangeReason` raw value).
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_route_changed(reason: i32) {
    crate::cmd::events::emit(crate::cmd::MobileEvent::Route { reason });
}

/// The system sent a memory warning.
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_memory_warning() {
    crate::lifecycle::memory_warning();
}

/// The share extension put files in the App Group inbox.
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_inbox_changed() {
    crate::cmd::events::emit(crate::cmd::MobileEvent::InboxChanged);
}

/// Why a pairing scan ended without a code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QrError {
    /// Camera access is off: the UI offers "Allow camera in Settings".
    Denied,
    /// No usable camera (or nothing to present the scanner from).
    Unavailable,
    /// The user closed the scanner.
    Cancelled,
}

/// Swift marks an error as this prefix + a code (see `GhiSync.swift`).
const QR_ERROR_PREFIX: &str = "\u{1}ghi-error:";

/// Decodes what Swift sent to `ghi_ios_qr_scanned`.
fn parse_qr(text: String) -> Result<String, QrError> {
    match text.strip_prefix(QR_ERROR_PREFIX) {
        None => Ok(text),
        Some("denied") => Err(QrError::Denied),
        Some("cancelled") => Err(QrError::Cancelled),
        Some(_) => Err(QrError::Unavailable),
    }
}

type QrChannel = (
    std::sync::mpsc::Sender<Result<String, QrError>>,
    std::sync::Mutex<std::sync::mpsc::Receiver<Result<String, QrError>>>,
);

fn qr_channel() -> &'static QrChannel {
    static CH: std::sync::OnceLock<QrChannel> = std::sync::OnceLock::new();
    CH.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        (tx, std::sync::Mutex::new(rx))
    })
}

/// Waits up to `timeout` for the next scan result (a code or a `QrError`).
/// The text carries a secret: callers must not log it. One consumer (the sync
/// service) reads; a result nobody asked for waits in the queue, so
/// `qr_drain` first when starting a scan.
#[allow(dead_code)] // consumed by the sync service (15-J)
pub fn qr_recv(timeout: std::time::Duration) -> Option<Result<String, QrError>> {
    let rx = qr_channel().1.lock().unwrap_or_else(|p| p.into_inner());
    rx.recv_timeout(timeout).ok()
}

/// Drops results left over from an earlier scan.
#[allow(dead_code)] // consumed by the sync service (15-J)
pub fn qr_drain() {
    let rx = qr_channel().1.lock().unwrap_or_else(|p| p.into_inner());
    while rx.try_recv().is_ok() {}
}

/// The desktops Swift's Bonjour browse currently sees (LAN addresses only).
pub fn pushed_discovery() -> &'static ghi_net::lan::PushedDiscovery {
    static D: std::sync::OnceLock<ghi_net::lan::PushedDiscovery> = std::sync::OnceLock::new();
    D.get_or_init(ghi_net::lan::PushedDiscovery::new)
}

/// Every `ip:port` of `[{"addrs":[…]}, …]`; malformed entries are skipped.
fn parse_browse(json: &str) -> Vec<std::net::SocketAddr> {
    #[derive(serde::Deserialize)]
    struct Svc {
        #[serde(default)]
        addrs: Vec<String>,
    }
    let Ok(list) = serde_json::from_str::<Vec<Svc>>(json) else {
        return Vec::new();
    };
    list.into_iter()
        .flat_map(|s| s.addrs)
        .filter_map(|a| a.parse().ok())
        .collect()
}

/// Swift scanned a pairing code, or the scan failed (see [`QrError`]). The text
/// carries a secret, so it is never logged; the sync service reads it with
/// [`qr_recv`].
///
/// # Safety
/// `text` is NULL or a NUL-terminated string valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghi_ios_qr_scanned(text: *const std::ffi::c_char) {
    // SAFETY: the caller guarantees a valid NUL-terminated string (or NULL).
    if let Some(text) = unsafe { c_text(text) } {
        let _ = qr_channel().0.send(parse_qr(text));
    }
}

/// Swift's Bonjour browse changed: `json` lists the visible services as
/// `[{"addrs":["ip:port",…]}, …]`. Non-LAN addresses are dropped by
/// `PushedDiscovery`.
///
/// # Safety
/// `json` is NULL or a NUL-terminated string valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghi_ios_browse_found(json: *const std::ffi::c_char) {
    // SAFETY: the caller guarantees a valid NUL-terminated string (or NULL).
    if let Some(json) = unsafe { c_text(json) } {
        pushed_discovery().set(parse_browse(&json));
    }
}

/// Copies a C string argument; `None` for NULL or invalid UTF-8.
///
/// # Safety
/// `p` is NULL or a NUL-terminated string valid for the call.
unsafe fn c_text(p: *const std::ffi::c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: non-null and NUL-terminated per the contract.
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_callbacks_tolerate_null_and_bad_text() {
        // SAFETY: NULL and a valid C string.
        unsafe {
            ghi_ios_qr_scanned(std::ptr::null());
            ghi_ios_browse_found(c"[]".as_ptr());
            ghi_ios_browse_found(c"not json".as_ptr());
            assert_eq!(c_text(c"abc".as_ptr()).as_deref(), Some("abc"));
            assert!(c_text(c"\xff".as_ptr()).is_none());
            assert!(c_text(std::ptr::null()).is_none());
        }
    }

    #[test]
    fn qr_results_and_errors_are_told_apart() {
        assert_eq!(
            parse_qr("ghira://pair?x".into()),
            Ok("ghira://pair?x".into())
        );
        assert_eq!(
            parse_qr("\u{1}ghi-error:denied".into()),
            Err(QrError::Denied)
        );
        assert_eq!(
            parse_qr("\u{1}ghi-error:cancelled".into()),
            Err(QrError::Cancelled)
        );
        assert_eq!(
            parse_qr("\u{1}ghi-error:other".into()),
            Err(QrError::Unavailable)
        );
    }

    #[test]
    fn browse_json_keeps_only_parsable_addresses() {
        let got =
            parse_browse(r#"[{"addrs":["192.168.1.5:7000","junk"]},{"addrs":["10.0.0.2:1"]},{}]"#);
        assert_eq!(got.len(), 2);
        assert!(parse_browse("nope").is_empty());
    }

    /// The Swift enum and `ActivityPhase` list the same cases with the same numbers.
    #[test]
    fn activity_phase_matches_the_swift_enum() {
        let swift = include_str!("../../../../native/ios/Shared/ActivityPhase.swift");
        let rust = [
            ("loading", ActivityPhase::Loading),
            ("live", ActivityPhase::Live),
            ("locked", ActivityPhase::Locked),
            ("catchingUp", ActivityPhase::CatchingUp),
            ("hot", ActivityPhase::Hot),
            ("interrupted", ActivityPhase::Interrupted),
            ("finishing", ActivityPhase::Finishing),
            ("done", ActivityPhase::Done),
            ("recordOnly", ActivityPhase::RecordOnly),
            ("paused", ActivityPhase::Paused),
        ];
        for (name, phase) in rust {
            let line = format!("case {name} = {}\n", phase as i32);
            assert!(swift.contains(&line), "Swift has no `{}`", line.trim());
        }
        assert_eq!(
            swift.matches("    case ").count() - swift.matches("        case ").count(),
            rust.len()
        );
    }

    /// Off iOS there is no call: an interruption shows as a plain pause.
    #[test]
    fn an_interruption_without_a_call_is_a_pause() {
        assert_eq!(
            activity_code(Phase::Interrupted),
            ActivityPhase::Paused as i32
        );
        assert_eq!(activity_code(Phase::Live), ActivityPhase::Live as i32);
    }

    #[test]
    fn every_session_phase_maps() {
        assert_eq!(ActivityPhase::from(Phase::Interrupted) as i32, 5);
        assert_eq!(ActivityPhase::from(Phase::Done) as i32, 7);
    }
}
