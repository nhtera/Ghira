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

fn phase_code(p: Phase) -> i32 {
    match p {
        Phase::Loading => 0,
        Phase::Live => 1,
        Phase::Locked => 2,
        Phase::CatchingUp => 3,
        Phase::Hot => 4,
        Phase::Interrupted => 5,
        Phase::Finishing => 6,
        Phase::Done => 7,
        Phase::RecordOnly => 8,
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

pub fn activity_start(phase: Phase) {
    let _code = phase_code(phase);
    // SAFETY: plain C call into Swift.
    #[cfg(target_os = "ios")]
    unsafe {
        swift::ghi_swift_activity_start(_code)
    }
}

/// Updates the Live Activity, or ends it once the session is `finished`
/// (capture stopped and the engine done).
pub fn activity_update(phase: Phase, marks: u32, finished: bool) {
    let _args = (phase_code(phase), marks, finished);
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
    if let Some(s) = session::current() {
        s.shared.gate.suspend();
        s.activity_update();
    }
}

/// The app entered the background (main thread, never blocks). Returns
/// `false` if an engine step overlapped the transition: the engine drops its
/// results, reloads the models and redoes that audio.
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_entered_background() -> bool {
    match session::current() {
        Some(s) => s.shared.gate.entered_background(),
        None => true,
    }
}

/// The app is active again: the engine may use the GPU and catches up.
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_resume() {
    if let Some(s) = session::current() {
        s.shared.gate.resume();
        s.activity_update();
    }
}

/// `ProcessInfo.thermalState` changed (0 nominal … 3 critical).
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_thermal_changed(state: i32) {
    if let Some(s) = session::current() {
        s.shared.set_thermal(state);
        s.activity_update();
    }
}

/// An audio interruption began (`true`) or ended (`false`).
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_interruption(began: bool) {
    if let Some(s) = session::current()
        && !s.shared.capture_done()
    {
        s.interrupted(began);
    }
}

/// The Live Activity's Stop intent.
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_stop_requested() {
    if let Some(s) = session::current() {
        s.stop();
    }
}

/// The Live Activity's Mark intent.
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_mark_requested() {
    if let Some(s) = session::current()
        && !s.shared.capture_done()
    {
        s.mark();
    }
}

// --- Rust → Swift wrappers (no-ops off iOS) ---------------------------------

/// Wrappers that later slices wire up (16-D session, 16-E native, 16-G services).
#[allow(dead_code)]
mod wrappers {
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
    crate::cmd::events::emit(crate::cmd::MobileEvent::MemoryWarning);
}

/// The share extension put files in the App Group inbox.
#[unsafe(no_mangle)]
pub extern "C" fn ghi_ios_inbox_changed() {
    crate::cmd::events::emit(crate::cmd::MobileEvent::InboxChanged);
}
