// SPDX-License-Identifier: Apache-2.0
//! App lock (doc 02 §M, D12): with the setting on, the app opens locked,
//! locks again after the chosen idle time and after the Mac sleeps, and
//! unlocks with Touch ID or the Mac's password (LocalAuthentication).
//!
//! While locked, `Core::store()` refuses, so no command reads or changes a
//! meeting, and transcript and speaker events don't reach the webview; the
//! main window and the popover show only the lock screen. A recording, the
//! jobs and queued imports carry on (`store_even_locked`); the
//! mini-recorder keeps its timer, pause and stop, never the words.
//!
//! The store's master key stays in the login keychain in this build. Binding
//! it to user presence (the data-protection keychain) needs the signed app's
//! keychain entitlements: see SECURITY.md (owner decision, phase 12 F2).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::core::Core;

/// The lock changed (every window follows it).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct LockChanged {
    pub locked: bool,
}

#[derive(Default)]
pub struct Lock {
    /// The launch check ran (the lock engaged if the setting is on).
    checked: AtomicBool,
}

fn emit<R: tauri::Runtime>(app: &AppHandle<R>, locked: bool) {
    let _ = LockChanged { locked }.emit(app);
}

/// Locks now (if the setting is on). Returns whether the app is locked, or
/// an error when the settings can't be read (the store isn't open yet).
pub fn lock<R: tauri::Runtime>(app: &AppHandle<R>, core: &Core) -> Result<bool, String> {
    let on = crate::system::load_settings(core)?.app_lock;
    if on && !core.locked() {
        core.set_locked(true);
        emit(app, true);
    }
    Ok(core.locked())
}

impl Lock {
    /// The launch lock, once the settings can be read (retried until then).
    fn launch(&self, app: &AppHandle, core: &Core) -> Result<bool, String> {
        if self.checked.load(Ordering::Acquire) {
            return Ok(core.locked());
        }
        let locked = lock(app, core)?;
        self.checked.store(true, Ordering::Release);
        Ok(locked)
    }
}

/// Error codes the UI turns into words (`system.locked.errors.*`).
pub const NO_AUTH: &str = "noAuthMethod";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const NO_ANSWER: &str = "noAnswer";
pub const NOT_CONFIRMED: &str = "notConfirmed";

/// One prompt at a time: a second window asking waits for the first answer.
static PROMPT: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Asks for Touch ID or the Mac's password; blocks until answered.
#[cfg(target_os = "macos")]
fn authenticate(reason: &str) -> Result<bool, String> {
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    let ctx = unsafe { LAContext::new() };
    // SAFETY: a fresh context; the policy constant is a valid LAPolicy.
    unsafe { ctx.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthentication) }
        .map_err(|_| NO_AUTH.to_string())?;
    let (tx, rx) = std::sync::mpsc::channel();
    let reply = RcBlock::new(move |ok: Bool, _err: *mut NSError| {
        let _ = tx.send(ok.as_bool());
    });
    let reason = NSString::from_str(reason);
    // SAFETY: the context, reason and block outlive the call; the reply runs
    // once on a system queue and only sends on the channel.
    unsafe {
        ctx.evaluatePolicy_localizedReason_reply(
            LAPolicy::DeviceOwnerAuthentication,
            &reason,
            &reply,
        )
    };
    match rx.recv_timeout(Duration::from_secs(300)) {
        Ok(ok) => Ok(ok),
        Err(_) => {
            // Take the prompt off the screen: a late answer must not count.
            // SAFETY: the context is alive; invalidate is safe at any time.
            unsafe { ctx.invalidate() };
            Err(NO_ANSWER.into())
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn authenticate(_reason: &str) -> Result<bool, String> {
    Err(NO_AUTH.into())
}

/// Asks once at a time across windows (see [`PROMPT`]).
fn confirm(reason: &str) -> Result<bool, String> {
    let _one = PROMPT.lock().unwrap_or_else(|e| e.into_inner());
    let reason: String = reason.chars().take(120).collect();
    authenticate(&reason)
}

/// The screen is locked (the user locked it or the screensaver did).
#[cfg(target_os = "macos")]
fn screen_locked() -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGSessionCopyCurrentDictionary() -> core_foundation::dictionary::CFDictionaryRef;
    }
    // SAFETY: a copy we own (create rule), or null outside a GUI session.
    let raw = unsafe { CGSessionCopyCurrentDictionary() };
    if raw.is_null() {
        return false;
    }
    // SAFETY: non-null, owned by us (Copy rule).
    let dict: CFDictionary<CFString, core_foundation::base::CFType> =
        unsafe { CFDictionary::wrap_under_create_rule(raw) };
    dict.find(CFString::from_static_string("CGSSessionScreenIsLocked"))
        .and_then(|v| v.downcast::<CFBoolean>())
        .is_some_and(bool::from)
}

#[cfg(not(target_os = "macos"))]
fn screen_locked() -> bool {
    false
}

/// The Mac slept between two ticks: wall-clock time ran ahead of the
/// monotonic clock (which stops during sleep) by more than 30 s.
fn slept(wall: Duration, mono: Duration) -> bool {
    wall.saturating_sub(mono) > Duration::from_secs(30)
}

/// Seconds since the last keyboard or mouse input anywhere on the Mac.
#[cfg(target_os = "macos")]
fn idle_seconds() -> f64 {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    }
    // kCGEventSourceStateCombinedSessionState, kCGAnyInputEventType.
    // SAFETY: a pure query with constant arguments.
    unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) }
}

#[cfg(not(target_os = "macos"))]
fn idle_seconds() -> f64 {
    0.0
}

/// Locks after the idle time, after a sleep (a wall-clock jump between two
/// ticks) and when the screen is locked.
pub fn spawn_watch(app: AppHandle) {
    const TICK: Duration = Duration::from_secs(3);
    // The launch lock as soon as the store opens (not only when the window
    // first asks).
    let first = app.clone();
    let _ = std::thread::Builder::new()
        .name("ghi-lock-launch".into())
        .spawn(move || {
            for _ in 0..600 {
                let core = first.state::<Arc<Core>>().inner().clone();
                let state = first.state::<Arc<Lock>>().inner().clone();
                if state.launch(&first, &core).is_ok() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    let _ = std::thread::Builder::new()
        .name("ghi-lock".into())
        .spawn(move || {
            let mut last_wall = SystemTime::now();
            let mut last_mono = Instant::now();
            loop {
                std::thread::sleep(TICK);
                let core = app.state::<Arc<Core>>().inner().clone();
                let wall = SystemTime::now();
                let jumped = slept(
                    wall.duration_since(last_wall).unwrap_or_default(),
                    last_mono.elapsed(),
                );
                last_wall = wall;
                last_mono = Instant::now();
                if core.locked() {
                    continue;
                }
                let Ok(s) = crate::system::load_settings(&core) else {
                    continue;
                };
                if !s.app_lock {
                    continue;
                }
                let idle = s.lock_after_minutes > 0
                    && idle_seconds() >= f64::from(s.lock_after_minutes) * 60.0;
                if jumped || idle || screen_locked() {
                    let _ = lock(&app, &core);
                }
            }
        });
}

/// Whether the app is locked. The first call after launch locks it when the
/// setting is on (nothing is shown before that).
#[tauri::command]
#[specta::specta]
pub async fn lock_state(
    app: AppHandle,
    core: crate::CoreState<'_>,
    state: tauri::State<'_, Arc<Lock>>,
) -> Result<bool, String> {
    let (core, state) = (core.inner().clone(), state.inner().clone());
    tauri::async_runtime::spawn_blocking(move || state.launch(&app, &core))
        .await
        .map_err(|e| e.to_string())?
}

/// "Lock now" (Settings, and the app menu's "Lock Ghira").
#[tauri::command]
#[specta::specta]
pub async fn lock_now(app: AppHandle, core: crate::CoreState<'_>) -> Result<bool, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || lock(&app, &core))
        .await
        .map_err(|e| e.to_string())?
}

/// Unlocks after Touch ID or the Mac's password. `reason` is shown in the
/// system prompt ("Ghira is trying to <reason>"). False: not confirmed.
#[tauri::command]
#[specta::specta]
pub async fn unlock(
    app: AppHandle,
    core: crate::CoreState<'_>,
    reason: String,
) -> Result<bool, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !core.locked() {
            return Ok(true);
        }
        let ok = confirm(&reason)?;
        // Another window's prompt may have unlocked meanwhile.
        if !core.locked() {
            return Ok(true);
        }
        if ok {
            core.set_locked(false);
            emit(&app, false);
            Ok(true)
        } else {
            Ok(false)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Turns the app lock on or off, or changes its idle time. Turning it on or
/// off asks first (on: so nobody locks themselves out; off: so nobody at an
/// unlocked Mac turns it off); changing the time while on doesn't.
#[tauri::command]
#[specta::specta]
pub async fn set_app_lock(
    app: AppHandle,
    core: crate::CoreState<'_>,
    on: bool,
    after_minutes: u32,
    reason: String,
) -> Result<crate::system::AppSettings, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let was_on = crate::system::load_settings(&core)?.app_lock;
        if on != was_on && !confirm(&reason)? {
            return Err(NOT_CONFIRMED.into());
        }
        crate::system::patch_settings(
            &app,
            &core,
            crate::system::SettingsPatch {
                app_lock: Some(on),
                lock_after_minutes: Some(after_minutes.min(240)),
                ..Default::default()
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wall_clock_jump_means_the_mac_slept() {
        let s = Duration::from_secs;
        assert!(slept(s(400), s(3)), "slept ~6 min");
        assert!(!slept(s(3), s(3)));
        assert!(!slept(s(25), s(3)), "under 30 s of drift");
        assert!(!slept(s(0), s(3)), "clock set back");
    }

    #[test]
    fn the_window_cannot_switch_the_lock_through_a_settings_patch() {
        let p: crate::system::SettingsPatch =
            serde_json::from_value(serde_json::json!({"appLock": false, "lockAfterMinutes": 0}))
                .unwrap();
        assert_eq!((p.app_lock, p.lock_after_minutes), (None, None));
    }
}
