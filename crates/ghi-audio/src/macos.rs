// SPDX-License-Identifier: Apache-2.0
//! macOS capture: bindings to the Swift `GhiAudioMac` package (built and linked
//! by `build.rs`). The C ABI is `native/macos/GhiAudioMac/include/ghi_audio_mac.h`;
//! keep the declarations below in sync with it.
//!
//! [`MacCapture::start`] opens the devices and returns one [`RingConsumer`] per
//! captured track for a [`crate::pipeline::Pipeline`], plus a channel of
//! [`CaptureEvent`]s. The audio callback only pushes into the rings.

use std::cell::UnsafeCell;
use std::ffi::{CStr, c_char, c_void};
use std::fmt;
use std::sync::mpsc;

use crate::detect::{AudioProcess, parse_processes};
use crate::ring::{RingConsumer, RingProducer, ring};
use crate::{CaptureEvent, Route, Track};

const ABI_VERSION: u32 = 1;

const CAPTURE_MIC: u32 = 0x1;
const CAPTURE_SYSTEM: u32 = 0x2;

const EV_TRACK_STARTED: u32 = 1;
const EV_ROUTE_CHANGED: u32 = 2;
const EV_MIC_DEVICE_CHANGED: u32 = 3;
const EV_SYSTEM_RESTARTED: u32 = 4;
const EV_TRACK_LOST: u32 = 5;
const EV_SLEEP: u32 = 6;
const EV_WAKE: u32 = 7;
const EV_ERROR: u32 = 8;

type AudioFn = extern "C" fn(*mut c_void, u32, *const f32, u32, f64, u64);
type EventFn = extern "C" fn(*mut c_void, u32, i64, *const c_char);

unsafe extern "C" {
    fn ghi_mac_abi_version() -> u32;
    fn ghi_mac_start(
        flags: u32,
        tap_pids: *const i32,
        n_tap_pids: u32,
        audio: AudioFn,
        event: EventFn,
        ctx: *mut c_void,
        out_session: *mut *mut c_void,
    ) -> i32;
    fn ghi_mac_pause(session: *mut c_void) -> i32;
    fn ghi_mac_resume(session: *mut c_void) -> i32;
    fn ghi_mac_stop(session: *mut c_void);
    fn ghi_mac_mic_permission() -> i32;
    fn ghi_mac_request_mic_permission() -> i32;
    fn ghi_mac_route(output_kind: *mut u32, input_bluetooth: *mut u32) -> i32;
    fn ghi_mac_audio_processes() -> *mut c_char;
    fn ghi_mac_free(s: *mut c_char);
}

/// A failed capture call. `code` is a `GHI_MAC_ERR_*` value or a negated
/// Core Audio `OSStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacError {
    pub op: &'static str,
    pub code: i32,
}

impl MacError {
    pub const INVALID: i32 = -1;
    pub const UNSUPPORTED: i32 = -2;
    pub const MIC_PERMISSION: i32 = -3;
    pub const NO_DEVICE: i32 = -4;
    pub const TAP: i32 = -5;
    pub const SYSTEM_PERMISSION: i32 = -6;

    fn check(op: &'static str, code: i32) -> Result<(), MacError> {
        if code == 0 {
            Ok(())
        } else {
            Err(MacError { op, code })
        }
    }
}

impl fmt::Display for MacError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.code {
            Self::INVALID => "invalid argument or state",
            Self::UNSUPPORTED => "needs macOS 14.2 or later",
            Self::MIC_PERMISSION => "microphone access is denied",
            Self::NO_DEVICE => "no audio device",
            Self::TAP => "system audio tap failed",
            Self::SYSTEM_PERMISSION => "System Audio Recording access is denied",
            _ => "Core Audio error",
        };
        write!(f, "{}: {what} ({})", self.op, self.code)
    }
}

impl std::error::Error for MacError {}

/// Microphone permission (`AVAuthorizationStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicPermission {
    Undetermined,
    Restricted,
    Denied,
    Authorized,
}

impl MicPermission {
    fn from_code(code: i32) -> MicPermission {
        match code {
            3 => MicPermission::Authorized,
            2 => MicPermission::Denied,
            1 => MicPermission::Restricted,
            _ => MicPermission::Undetermined,
        }
    }
}

/// Whether the linked Swift library speaks the ABI these bindings expect.
pub fn abi_matches() -> bool {
    // SAFETY: no arguments, no state.
    unsafe { ghi_mac_abi_version() == ABI_VERSION }
}

pub fn mic_permission() -> MicPermission {
    // SAFETY: no arguments.
    MicPermission::from_code(unsafe { ghi_mac_mic_permission() })
}

/// Shows the system prompt when undetermined and waits for the answer.
/// Do not call it on the main thread of an app.
pub fn request_mic_permission() -> MicPermission {
    // SAFETY: no arguments; blocks until the user answers.
    MicPermission::from_code(unsafe { ghi_mac_request_mic_permission() })
}

/// The current output route and whether the default input is a Bluetooth
/// headset (hands-free profile).
pub fn route() -> Result<(Route, bool), MacError> {
    let (mut kind, mut bluetooth) = (0u32, 0u32);
    // SAFETY: both pointers are valid for writes.
    MacError::check("route", unsafe { ghi_mac_route(&mut kind, &mut bluetooth) })?;
    Ok((Route::from_code(kind), bluetooth != 0))
}

/// Processes known to Core Audio, for meeting auto-detect.
pub fn audio_processes() -> Result<Vec<AudioProcess>, MacError> {
    // SAFETY: returns a malloc'd NUL-terminated string or NULL.
    let ptr = unsafe { ghi_mac_audio_processes() };
    if ptr.is_null() {
        return Err(MacError {
            op: "audio_processes",
            code: MacError::INVALID,
        });
    }
    // SAFETY: non-null, NUL-terminated, owned by us until ghi_mac_free.
    let json = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: frees the string returned above, exactly once.
    unsafe { ghi_mac_free(ptr) };
    parse_processes(&json).map_err(|_| MacError {
        op: "audio_processes",
        code: MacError::INVALID,
    })
}

/// What to capture.
#[derive(Debug, Clone, Default)]
pub struct CaptureConfig {
    pub mic: bool,
    pub system: bool,
    /// Tap only these processes (per-app capture); empty = every process
    /// except this one.
    pub tap_pids: Vec<i32>,
}

/// Ring capacity per track: 4 s at 48 kHz, enough to ride out a stalled
/// pipeline thread without dropping samples.
const RING_SAMPLES: usize = 4 * 48_000;

/// Shared with the Swift callbacks through `ctx`.
struct Ctx {
    /// One producer per track. Swift serializes the calls for one track, and
    /// each producer is only touched by its own track's calls.
    producers: [UnsafeCell<Option<RingProducer>>; 2],
    events: mpsc::Sender<CaptureEvent>,
}

// SAFETY: see `producers`: each cell is accessed by one serialized call chain;
// `mpsc::Sender` is Sync.
unsafe impl Sync for Ctx {}

/// A running capture session. Dropping it stops the devices.
pub struct MacCapture {
    session: *mut c_void,
    ctx: *mut Ctx,
}

// SAFETY: the session handle may be used from any thread (not from inside a
// callback, which this type never does).
unsafe impl Send for MacCapture {}

/// What [`MacCapture::start`] hands back.
pub struct Started {
    pub capture: MacCapture,
    pub mic: Option<RingConsumer>,
    pub system: Option<RingConsumer>,
    pub events: mpsc::Receiver<CaptureEvent>,
}

impl MacCapture {
    pub fn start(cfg: &CaptureConfig) -> Result<Started, MacError> {
        if !abi_matches() {
            return Err(MacError {
                op: "start",
                code: MacError::UNSUPPORTED,
            });
        }
        if !cfg.mic && !cfg.system {
            return Err(MacError {
                op: "start",
                code: MacError::INVALID,
            });
        }
        let (mic_tx, mic_rx) = split(cfg.mic);
        let (sys_tx, sys_rx) = split(cfg.system);
        let (events_tx, events) = mpsc::channel();
        let ctx = Box::into_raw(Box::new(Ctx {
            producers: [UnsafeCell::new(mic_tx), UnsafeCell::new(sys_tx)],
            events: events_tx,
        }));
        let flags =
            if cfg.mic { CAPTURE_MIC } else { 0 } | if cfg.system { CAPTURE_SYSTEM } else { 0 };
        let mut session = std::ptr::null_mut();
        // SAFETY: the pid slice outlives the call; `ctx` stays valid until
        // ghi_mac_stop has returned (Drop) or start failed (below).
        let status = unsafe {
            ghi_mac_start(
                flags,
                cfg.tap_pids.as_ptr(),
                cfg.tap_pids.len() as u32,
                on_audio,
                on_event,
                ctx.cast(),
                &mut session,
            )
        };
        if let Err(e) = MacError::check("start", status) {
            // SAFETY: start failed, so Swift keeps no reference to `ctx`.
            drop(unsafe { Box::from_raw(ctx) });
            return Err(e);
        }
        Ok(Started {
            capture: MacCapture { session, ctx },
            mic: mic_rx,
            system: sys_rx,
            events,
        })
    }

    /// Stops the devices (the mic indicator turns off) until [`Self::resume`].
    pub fn pause(&self) -> Result<(), MacError> {
        // SAFETY: live session, not called from a callback.
        MacError::check("pause", unsafe { ghi_mac_pause(self.session) })
    }

    pub fn resume(&self) -> Result<(), MacError> {
        // SAFETY: live session, not called from a callback.
        MacError::check("resume", unsafe { ghi_mac_resume(self.session) })
    }
}

impl Drop for MacCapture {
    fn drop(&mut self) {
        // SAFETY: after ghi_mac_stop returns no callback runs again, so the
        // context can be freed.
        unsafe {
            ghi_mac_stop(self.session);
            drop(Box::from_raw(self.ctx));
        }
    }
}

/// What a short system-audio capture heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemProbe {
    /// Audio came through the tap: access is on.
    Heard,
    /// Only digital silence: nothing was playing, or access is denied (a
    /// denied tap delivers zeros, and macOS cannot be asked which).
    Silent,
    /// Starting the capture was refused for lack of access.
    Denied,
}

/// Captures the system's audio for `listen` and drops it (nothing is kept):
/// the first start shows macOS's "screen & system audio recording" prompt.
/// `on_started` runs once the tap is running (play the test sound there).
/// Blocks; do not call it on the main thread of an app.
pub fn probe_system_audio(
    listen: std::time::Duration,
    on_started: impl FnOnce(),
) -> Result<SystemProbe, MacError> {
    let started = match MacCapture::start(&CaptureConfig {
        mic: false,
        system: true,
        tap_pids: Vec::new(),
    }) {
        Ok(s) => s,
        Err(e) if e.code == MacError::SYSTEM_PERMISSION => return Ok(SystemProbe::Denied),
        Err(e) => return Err(e),
    };
    on_started();
    let mut ring = started.system;
    let mut block = Vec::new();
    let mut peak = 0.0f32;
    let end = std::time::Instant::now() + listen;
    while std::time::Instant::now() < end {
        if let Some(r) = ring.as_mut() {
            while r.pop_into(&mut block).is_some() {
                peak = block.iter().fold(peak, |p, s| p.max(s.abs()));
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    drop(started.capture);
    Ok(if peak > 1e-4 {
        SystemProbe::Heard
    } else {
        SystemProbe::Silent
    })
}

fn split(on: bool) -> (Option<RingProducer>, Option<RingConsumer>) {
    if on {
        let (tx, rx) = ring(RING_SAMPLES);
        (Some(tx), Some(rx))
    } else {
        (None, None)
    }
}

/// Realtime audio callback: push into the track's ring, nothing else.
extern "C" fn on_audio(
    ctx: *mut c_void,
    track: u32,
    samples: *const f32,
    frames: u32,
    rate: f64,
    host_ns: u64,
) {
    let Some(track) = Track::from_index(track) else {
        return;
    };
    if ctx.is_null() || samples.is_null() {
        return;
    }
    // SAFETY: `ctx` is the live Ctx; calls for this track are serialized, so
    // this is the only access to its producer cell.
    let producer = unsafe { &mut *(*ctx.cast::<Ctx>()).producers[track.index()].get() };
    if let Some(producer) = producer {
        // SAFETY: Swift guarantees `frames` samples, valid during the call.
        let block = unsafe { std::slice::from_raw_parts(samples, frames as usize) };
        producer.push(block, rate, host_ns);
    }
}

/// Event callback (non-realtime queue): translate and forward.
extern "C" fn on_event(ctx: *mut c_void, kind: u32, code: i64, detail: *const c_char) {
    if ctx.is_null() {
        return;
    }
    let detail = if detail.is_null() {
        String::new()
    } else {
        // SAFETY: NUL-terminated UTF-8, valid during the call.
        unsafe { CStr::from_ptr(detail) }
            .to_string_lossy()
            .into_owned()
    };
    let Some(event) = translate(kind, code, detail) else {
        return;
    };
    // SAFETY: `ctx` is the live Ctx; the sender is Sync.
    let ctx = unsafe { &*ctx.cast::<Ctx>() };
    // The receiver may be gone while shutting down; nothing to do then.
    let _ = ctx.events.send(event);
}

fn translate(kind: u32, code: i64, detail: String) -> Option<CaptureEvent> {
    let track = || Track::from_index(u32::try_from(code).ok()?);
    Some(match kind {
        EV_TRACK_STARTED => CaptureEvent::TrackStarted {
            track: track()?,
            device: detail,
        },
        EV_ROUTE_CHANGED => CaptureEvent::RouteChanged {
            route: Route::from_code(code as u32),
            input_bluetooth_hfp: code & 0x100 != 0,
        },
        EV_MIC_DEVICE_CHANGED => CaptureEvent::MicDeviceChanged { device: detail },
        EV_SYSTEM_RESTARTED => CaptureEvent::SystemRestarted,
        EV_TRACK_LOST => CaptureEvent::TrackLost { track: track()? },
        EV_SLEEP => CaptureEvent::Sleep,
        EV_WAKE => CaptureEvent::Wake,
        EV_ERROR => CaptureEvent::Error { code, detail },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_and_speaks_our_abi() {
        assert!(abi_matches());
    }

    #[test]
    fn lists_audio_processes_and_route() {
        // Needs no permission; fails only if Core Audio itself does.
        let procs = audio_processes().unwrap();
        assert!(procs.iter().all(|p| p.pid > 0));
        // Headless CI Macs may have no output device at all.
        if let Err(e) = route() {
            assert_eq!(e.code, MacError::NO_DEVICE, "{e}");
        }
    }

    #[test]
    fn translates_events() {
        assert_eq!(
            translate(EV_ROUTE_CHANGED, 0x103, String::new()),
            Some(CaptureEvent::RouteChanged {
                route: Route::Bluetooth,
                input_bluetooth_hfp: true
            })
        );
        assert_eq!(
            translate(EV_TRACK_STARTED, 1, "Speakers".into()),
            Some(CaptureEvent::TrackStarted {
                track: Track::System,
                device: "Speakers".into()
            })
        );
        assert_eq!(translate(EV_TRACK_LOST, 7, String::new()), None);
        assert_eq!(translate(99, 0, String::new()), None);
    }
}
