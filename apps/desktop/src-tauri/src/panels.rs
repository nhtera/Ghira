// SPDX-License-Identifier: Apache-2.0
//! The menu-bar popover, the mini-recorder and the meeting-detection prompt
//! (D2, D4): small windows on routes of the one frontend bundle (`#/popover`,
//! `#/mini`, `#/detect`).
//!
//! On macOS they are non-activating panels (NSPanel): they float over other
//! apps (a full-screen Zoom included). Only the popover takes keyboard focus
//! (it is opened by a click); the mini-recorder and the detection prompt are
//! shown without it, so typing in the call never lands in them. They are
//! opaque with native rounded corners (no private API for transparency).
//!
//! AppKit is main-thread only: every panel operation runs through
//! `on_main`, whatever thread asks (the detection poller, a shortcut, a
//! command).
//!
//! The mini-recorder opens as a pill and asks to be left out of screen
//! capture (`sharingType = none`); macOS 15+ ScreenCaptureKit ignores that
//! (tauri#14200), so it is best effort and the UI never promises it.
//! Elsewhere (Windows, phase 13) they are plain always-on-top windows.

// `tauri_panel!`'s event syntax needs an explicit `-> ()`.
#![allow(clippy::unused_unit)]

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl};
use tauri_specta::Event;

use crate::system::MeetingDetected;

pub const POPOVER: &str = "popover";
pub const MINI: &str = "mini";
pub const DETECT: &str = "detect";

const POPOVER_SIZE: (f64, f64) = (340.0, 440.0);
const MINI_FULL: (f64, f64) = (360.0, 120.0);
const MINI_PILL: (f64, f64) = (180.0, 44.0);
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const DETECT_SIZE: (f64, f64) = (340.0, 150.0);
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const CORNER_RADIUS: f64 = 12.0;

#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    panel!(GhiraPanel {
        config: {
            can_become_key_window: true,
            can_become_main_window: false,
            is_floating_panel: true
        }
    })

    panel_event!(PopoverEvents {
        window_did_resign_key(notification: &NSNotification) -> ()
    })
}

/// Runs `f` on the main thread (directly when already there).
fn on_main(app: &AppHandle, f: impl FnOnce(&AppHandle) + Send + 'static) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || f(&handle));
}

fn url(route: &str) -> WebviewUrl {
    WebviewUrl::App(format!("index.html#{route}").into())
}

#[cfg(target_os = "macos")]
fn build_panel(
    app: &AppHandle,
    label: &str,
    route: &str,
    size: (f64, f64),
    position: Option<(f64, f64)>,
) -> tauri::Result<()> {
    use tauri_nspanel::{CollectionBehavior, PanelBuilder, PanelLevel, StyleMask};
    let mut b = PanelBuilder::<_, GhiraPanel>::new(app, label)
        .url(url(route))
        .level(PanelLevel::Floating)
        .size(tauri::Size::Logical(LogicalSize::new(size.0, size.1)))
        .style_mask(StyleMask::empty().nonactivating_panel().borderless())
        .collection_behavior(
            CollectionBehavior::new()
                .can_join_all_spaces()
                .full_screen_auxiliary(),
        )
        .corner_radius(CORNER_RADIUS)
        .has_shadow(true)
        .hides_on_deactivate(false)
        .no_activate(true)
        .with_window(|w| {
            // Hidden until shown as a panel: never a normal window on screen.
            w.decorations(false)
                .visible(false)
                .focused(false)
                .skip_taskbar(true)
                .on_new_window(|_url, _features| tauri::webview::NewWindowResponse::Deny)
        });
    if let Some((x, y)) = position {
        b = b.position(tauri::Position::Logical(LogicalPosition::new(x, y)));
    }
    b.build()?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn build_panel(
    app: &AppHandle,
    label: &str,
    route: &str,
    size: (f64, f64),
    position: Option<(f64, f64)>,
) -> tauri::Result<()> {
    let mut b = tauri::WebviewWindowBuilder::new(app, label, url(route))
        .inner_size(size.0, size.1)
        .resizable(false)
        .decorations(false)
        .visible(false)
        .focused(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .on_new_window(|_url, _features| tauri::webview::NewWindowResponse::Deny);
    if let Some((x, y)) = position {
        b = b.position(x, y);
    }
    b.build()?;
    Ok(())
}

/// Shows a panel; `key`: also take the keyboard (the popover only). Main thread.
fn show(app: &AppHandle, label: &str, key: bool) {
    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::ManagerExt;
        if let Ok(p) = app.get_webview_panel(label) {
            if key {
                p.show_and_make_key();
            } else {
                p.show();
            }
            return;
        }
    }
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.show();
        if key {
            let _ = w.set_focus();
        }
    }
}

/// Main thread.
fn hide(app: &AppHandle, label: &str) {
    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::ManagerExt;
        if let Ok(p) = app.get_webview_panel(label) {
            p.hide();
            return;
        }
    }
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.hide();
    }
}

/// Main thread.
fn visible(app: &AppHandle, label: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        use tauri_nspanel::ManagerExt;
        if let Ok(p) = app.get_webview_panel(label) {
            return p.is_visible();
        }
    }
    app.get_webview_window(label)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// The logical frame (x, y, w, h) of the monitor containing a point, for
/// keeping a panel on screen (mixed-DPI setups included).
fn monitor_frame(app: &AppHandle, at: (f64, f64)) -> Option<(f64, f64, f64, f64)> {
    let monitors = app.available_monitors().ok()?;
    monitors
        .iter()
        .map(|m| {
            let s = m.scale_factor();
            let p = m.position().to_logical::<f64>(s);
            let z = m.size().to_logical::<f64>(s);
            (p.x, p.y, z.width, z.height)
        })
        .find(|(x, y, w, h)| at.0 >= *x && at.0 < x + w && at.1 >= *y && at.1 < y + h)
        .or_else(|| {
            app.primary_monitor().ok().flatten().map(|m| {
                let s = m.scale_factor();
                let p = m.position().to_logical::<f64>(s);
                let z = m.size().to_logical::<f64>(s);
                (p.x, p.y, z.width, z.height)
            })
        })
}

/// Shows or hides the popover under the menu-bar icon (`anchor`: the icon's
/// bottom-centre in logical screen points), kept inside its screen.
pub fn toggle_popover(app: &AppHandle, anchor: (f64, f64)) {
    on_main(app, move |app| {
        let mut pos = (anchor.0 - POPOVER_SIZE.0 / 2.0, anchor.1 + 6.0);
        if let Some((x, _, w, _)) = monitor_frame(app, anchor) {
            pos.0 = pos.0.clamp(x + 8.0, x + w - POPOVER_SIZE.0 - 8.0);
        }
        if app.get_webview_window(POPOVER).is_none() {
            if build_panel(app, POPOVER, "/popover", POPOVER_SIZE, Some(pos)).is_err() {
                return;
            }
            #[cfg(target_os = "macos")]
            {
                // Clicking elsewhere closes it, like a menu.
                use tauri_nspanel::ManagerExt;
                if let Ok(p) = app.get_webview_panel(POPOVER) {
                    let events = PopoverEvents::new();
                    let handle = app.clone();
                    events.window_did_resign_key(move |_| hide(&handle, POPOVER));
                    p.set_event_handler(Some(events.as_ref()));
                }
            }
            show(app, POPOVER, true);
        } else if visible(app, POPOVER) {
            hide(app, POPOVER);
        } else {
            if let Some(w) = app.get_webview_window(POPOVER) {
                let _ =
                    w.set_position(tauri::Position::Logical(LogicalPosition::new(pos.0, pos.1)));
            }
            show(app, POPOVER, true);
        }
    });
}

/// Opens the mini-recorder as a pill (bottom-right of the main screen): the
/// latest words only show once the user expands it (screen-share privacy).
pub fn open_mini(app: &AppHandle) {
    on_main(app, |app| {
        if app.get_webview_window(MINI).is_none() {
            let pos = monitor_frame(app, (0.0, 0.0))
                .map(|(x, y, w, h)| (x + w - MINI_FULL.0 - 24.0, y + h - MINI_FULL.1 - 96.0));
            if build_panel(app, MINI, "/mini?compact=1", MINI_PILL, pos).is_err() {
                return;
            }
            exclude_from_capture(app, MINI);
        }
        show(app, MINI, false);
    });
}

/// Best effort: ask macOS to leave the window out of screen sharing. Main thread.
fn exclude_from_capture(app: &AppHandle, label: &str) {
    #[cfg(target_os = "macos")]
    if let Some(w) = app.get_webview_window(label)
        && let Ok(ns) = w.ns_window()
    {
        // SAFETY: Tauri's pointer to this live window's NSWindow, used on the
        // main thread (callers run through `on_main`) while the window exists.
        unsafe {
            (*(ns as *mut objc2_app_kit::NSWindow))
                .setSharingType(objc2_app_kit::NSWindowSharingType::None);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, label);
}

/// The meeting-detection prompt when the main window isn't in front (D2): a
/// non-activating panel at the top right, so the call keeps the focus. Built
/// once; later prompts reach it as an event.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn open_detect(app: &AppHandle, prompt: MeetingDetected) {
    on_main(app, move |app| {
        if app.get_webview_window(DETECT).is_some() {
            let _ = prompt.emit_to(app, DETECT);
        } else {
            // The first prompt rides in the URL (the page isn't listening yet).
            let enc = |v: &str| -> String {
                v.bytes()
                    .map(|b| match b {
                        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                            (b as char).to_string()
                        }
                        _ => format!("%{b:02X}"),
                    })
                    .collect()
            };
            let route = format!(
                "/detect?app={}&name={}&browser={}",
                enc(&prompt.app),
                enc(&prompt.app_name),
                u8::from(prompt.browser)
            );
            let pos = monitor_frame(app, (0.0, 0.0))
                .map(|(x, y, w, _)| (x + w - DETECT_SIZE.0 - 16.0, y + 40.0));
            if build_panel(app, DETECT, &route, DETECT_SIZE, pos).is_err() {
                return;
            }
        }
        show(app, DETECT, false);
    });
}

#[tauri::command]
#[specta::specta]
pub fn hide_popover(app: AppHandle) {
    on_main(&app, |app| hide(app, POPOVER));
}

/// The mini-recorder as a full card or a small pill.
#[tauri::command]
#[specta::specta]
pub fn set_mini_compact(app: AppHandle, compact: bool) -> Result<(), String> {
    let (w, h) = if compact { MINI_PILL } else { MINI_FULL };
    let win = app
        .get_webview_window(MINI)
        .ok_or("the mini-recorder is not open")?;
    win.set_size(tauri::Size::Logical(LogicalSize::new(w, h)))
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn close_mini(app: AppHandle) {
    on_main(&app, |app| hide(app, MINI));
}

/// Opens the mini-recorder from the live view ("Mini recorder").
#[tauri::command]
#[specta::specta]
pub fn open_mini_recorder(app: AppHandle) {
    open_mini(&app);
}

#[tauri::command]
#[specta::specta]
pub fn close_detect(app: AppHandle) {
    on_main(&app, |app| hide(app, DETECT));
}
