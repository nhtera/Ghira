// SPDX-License-Identifier: Apache-2.0
//! "Open Ghira at login": a login item through `SMAppService` on macOS (the
//! app must run from its bundle; an unbundled dev run reports the error and
//! the setting stays as the user chose), the `HKCU\...\Run` key on Windows (the system's own `reg.exe`).

use tauri::{AppHandle, Manager};

use crate::core::Core;

/// Makes the OS login item match the `openAtLogin` setting, touching it only
/// when it differs. Read even while the app is locked (a settings value, not
/// content); unreadable settings change nothing. Failures are logged, never
/// shown: the app works either way.
pub fn sync(app: &AppHandle) {
    let Ok(want) = crate::system::load_settings_even_locked(&app.state::<std::sync::Arc<Core>>())
        .map(|s| s.open_at_login)
    else {
        return;
    };
    if is_enabled() == Some(want) {
        return;
    }
    if let Err(e) = set(want) {
        log::warn!("login item: {e}");
    }
}

#[cfg(target_os = "macos")]
#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

/// `SMAppService.mainAppService`, or why there is none.
#[cfg(target_os = "macos")]
fn main_service() -> Result<objc2::rc::Retained<objc2::runtime::AnyObject>, &'static str> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    let class = AnyClass::get(c"SMAppService").ok_or("SMAppService needs macOS 13")?;
    // SAFETY: `+mainAppService` takes no arguments and returns an object.
    let svc: Option<Retained<AnyObject>> = unsafe { msg_send![class, mainAppService] };
    svc.ok_or("no login item for this app")
}

/// Whether the login item is registered (`None`: cannot tell).
#[cfg(target_os = "macos")]
pub fn is_enabled() -> Option<bool> {
    use objc2::msg_send;
    objc2::rc::autoreleasepool(|_| {
        let svc = main_service().ok()?;
        // SAFETY: `-status` takes no arguments and returns an NSInteger.
        let status: isize = unsafe { msg_send![&*svc, status] };
        // 0 not registered, 1 enabled, 2 needs the user's approval, 3 not found.
        match status {
            1 | 2 => Some(true),
            0 | 3 => Some(false),
            _ => None,
        }
    })
}

/// Registers or removes the login item.
#[cfg(target_os = "macos")]
pub fn set(on: bool) -> Result<(), String> {
    use objc2::msg_send;
    use objc2::rc::{Retained, autoreleasepool};
    use objc2_foundation::NSError;

    autoreleasepool(|_| {
        let svc = main_service()?;
        // SAFETY: both selectors take only the error out-parameter and return BOOL.
        let r: Result<(), Retained<NSError>> = unsafe {
            if on {
                msg_send![&*svc, registerAndReturnError: _]
            } else {
                msg_send![&*svc, unregisterAndReturnError: _]
            }
        };
        match r {
            Ok(()) => Ok(()),
            // Removing what was never added is not a failure.
            Err(_) if !on => Ok(()),
            Err(e) => Err(e.localizedDescription().to_string()),
        }
    })
}

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// The system's own `reg.exe` (never one found through `PATH`).
#[cfg(windows)]
fn reg() -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let mut c = std::process::Command::new(std::path::Path::new(&root).join(r"System32\reg.exe"));
    c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    c
}

#[cfg(windows)]
pub fn is_enabled() -> Option<bool> {
    reg()
        .args(["query", RUN_KEY, "/v", "Ghira"])
        .output()
        .ok()
        .map(|o| o.status.success())
}

#[cfg(windows)]
pub fn set(on: bool) -> Result<(), String> {
    let mut cmd = reg();
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        cmd.args(["add", RUN_KEY, "/v", "Ghira", "/t", "REG_SZ", "/f", "/d"])
            .arg(format!("\"{}\"", exe.display()));
    } else {
        cmd.args(["delete", RUN_KEY, "/v", "Ghira", "/f"]);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    // Deleting a value that is not there fails; that is the wanted state.
    if out.status.success() || !on {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn is_enabled() -> Option<bool> {
    None
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn set(_on: bool) -> Result<(), String> {
    Err("not available on this platform".into())
}
