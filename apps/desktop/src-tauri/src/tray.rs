// SPDX-License-Identifier: Apache-2.0
//! The menu-bar (mac) / tray (Windows) item (D2): its icon says idle,
//! recording (red, with the timer as its title), processing or attention;
//! a click opens the popover. Global shortcuts live here too: ⌘⇧R (Ctrl+
//! Shift+R) starts or stops recording from any app; ⌘M marks a moment, and
//! is held only while recording so it never takes Minimize from other apps.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use ghi_core::events::{ErrorKind, Event, SessionState};
use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::core::Core;
use crate::panels;

const TRAY: &str = "main";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum TrayState {
    Idle = 0,
    Recording = 1,
    Processing = 2,
    Attention = 3,
}

static STATE: AtomicU8 = AtomicU8::new(TrayState::Idle as u8);

fn icon(state: TrayState) -> (Image<'static>, bool) {
    let (bytes, template): (&[u8], bool) = match state {
        TrayState::Idle => (include_bytes!("../icons/tray/idle.png"), true),
        // Not a template: the dot stays red in light and dark menu bars.
        TrayState::Recording => (include_bytes!("../icons/tray/recording.png"), false),
        TrayState::Processing => (include_bytes!("../icons/tray/processing.png"), true),
        TrayState::Attention => (include_bytes!("../icons/tray/attention.png"), true),
    };
    (
        Image::from_bytes(bytes).expect("bundled tray icon"),
        template,
    )
}

fn set_state(app: &AppHandle, state: TrayState) {
    if STATE.swap(state as u8, Ordering::AcqRel) == state as u8 {
        return;
    }
    if let Some(t) = app.tray_by_id(TRAY) {
        let (img, template) = icon(state);
        let _ = t.set_icon(Some(img));
        let _ = t.set_icon_as_template(template);
        if state != TrayState::Recording {
            let _ = t.set_title(None::<&str>);
        }
    }
}

fn mark_shortcut() -> Shortcut {
    // Ctrl+M on Windows (Win+M minimizes every window).
    #[cfg(target_os = "macos")]
    let m = Modifiers::SUPER;
    #[cfg(not(target_os = "macos"))]
    let m = Modifiers::CONTROL;
    Shortcut::new(Some(m), Code::KeyM)
}

fn record_shortcut() -> Shortcut {
    #[cfg(target_os = "macos")]
    let m = Modifiers::SUPER | Modifiers::SHIFT;
    #[cfg(not(target_os = "macos"))]
    let m = Modifiers::CONTROL | Modifiers::SHIFT;
    Shortcut::new(Some(m), Code::KeyR)
}

/// The global-shortcut plugin with our handler.
pub fn shortcut_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let core = app.state::<Arc<Core>>().inner().clone();
            if *shortcut == mark_shortcut() {
                let _ = core.with_session(|s| s.mark());
            } else if *shortcut == record_shortcut() {
                let app = app.clone();
                // From another app: record without taking focus; the
                // mini-recorder (as a pill) shows what is happening.
                std::thread::spawn(move || {
                    if core.recording() {
                        let _ = core.stop();
                    } else if !core.busy() {
                        match core.start(ghi_core::live::Mode::Call, None, String::new()) {
                            Ok(_) => panels::open_mini(&app),
                            // Nothing on screen asked for it: say so in the menu bar.
                            Err(_) => set_state(&app, TrayState::Attention),
                        }
                    }
                });
            }
        })
        .build()
}

/// ⌘⇧R from any app: only after onboarding and while the setting is on (it
/// takes the chord from every app, e.g. hard reload in browsers).
pub fn sync_record_shortcut(app: &AppHandle) {
    let on = crate::system::load_settings(&app.state::<Arc<Core>>())
        .map(|s| s.onboarding_done && s.global_record_shortcut)
        .unwrap_or(false);
    let gs = app.global_shortcut();
    match (on, gs.is_registered(record_shortcut())) {
        (true, false) => {
            let _ = gs.register(record_shortcut());
        }
        (false, true) => {
            let _ = gs.unregister(record_shortcut());
        }
        _ => {}
    }
}

/// Whether the icon should show: the setting, but always while a recording
/// runs (the icon is the recording indicator). Settings are read even while
/// the app is locked.
fn icon_visible(show_setting: bool, recording: bool) -> bool {
    show_setting || recording
}

/// What `sync_visibility` last applied (0 hidden, 1 shown, 2 not yet).
static VISIBLE: AtomicU8 = AtomicU8::new(2);

/// Shows or hides the menu-bar / tray icon as the setting says (and the
/// recording needs). Cheap enough to run every second.
pub fn sync_visibility(app: &AppHandle) {
    let core = app.state::<Arc<Core>>();
    let Ok(s) = crate::system::load_settings_even_locked(&core) else {
        return;
    };
    let show = icon_visible(s.show_in_menu_bar, core.recording());
    if VISIBLE.swap(show as u8, Ordering::AcqRel) == show as u8 {
        return;
    }
    if let Some(t) = app.tray_by_id(TRAY) {
        let _ = t.set_visible(show);
    }
}

fn clock(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The tray item and its ticker: the recording timer as the title, and
/// between recordings whether jobs are running (asked from the store, so
/// imports, retries and models arriving count too).
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let (img, template) = icon(TrayState::Idle);
    TrayIconBuilder::with_id(TRAY)
        .icon(img)
        .icon_as_template(template)
        .tooltip("Ghira")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                rect,
                ..
            } = event
            {
                let app = tray.app_handle();
                // The scale of the screen the icon is on (mixed-DPI setups).
                let scale = app
                    .monitor_from_point(position.x, position.y)
                    .ok()
                    .flatten()
                    .or_else(|| app.primary_monitor().ok().flatten())
                    .map(|m| m.scale_factor())
                    .unwrap_or(1.0);
                let pos = rect.position.to_logical::<f64>(scale);
                let size = rect.size.to_logical::<f64>(scale);
                panels::toggle_popover(app, (pos.x + size.width / 2.0, pos.y + size.height));
            }
        })
        .build(app)?;
    let handle = app.clone();
    std::thread::Builder::new()
        .name("ghi-tray".into())
        .spawn(move || {
            let core = handle.state::<Arc<Core>>().inner().clone();
            // ⌘⇧R once the store (and so the settings) can be read.
            // The login item and the icon are not content: also when locked.
            if core.store_even_locked().is_ok() {
                sync_visibility(&handle);
                crate::login_item::sync(&handle);
            }
            if core.store().is_ok() {
                sync_record_shortcut(&handle);
            }
            let mut tick = 0u32;
            loop {
                std::thread::sleep(Duration::from_secs(1));
                tick = tick.wrapping_add(1);
                sync_visibility(&handle);
                let state = STATE.load(Ordering::Acquire);
                if state == TrayState::Recording as u8 {
                    if let (Ok(ms), Some(t)) =
                        (core.with_session(|s| s.now_ms()), handle.tray_by_id(TRAY))
                        && STATE.load(Ordering::Acquire) == TrayState::Recording as u8
                    {
                        let _ = t.set_title(Some(clock(ms)));
                    }
                } else if tick.is_multiple_of(3) && state != TrayState::Attention as u8 {
                    // Job state only (no content): also while the app is locked.
                    let busy = core
                        .store_even_locked()
                        .ok()
                        .and_then(|s| s.active_jobs().ok())
                        .is_some_and(|jobs| {
                            jobs.iter()
                                .any(|j| j.state == ghi_store::jobs::JobState::Running)
                        });
                    set_state(
                        &handle,
                        if busy {
                            TrayState::Processing
                        } else {
                            TrayState::Idle
                        },
                    );
                }
            }
        })?;
    Ok(())
}

/// Follows the core: tray state and the recording-only ⌘M shortcut.
pub fn on_event(app: &AppHandle, event: &Event) {
    match event {
        Event::StateChanged { state, .. } => match state {
            SessionState::Recording | SessionState::Paused => {
                set_state(app, TrayState::Recording);
                let mark_global = crate::system::load_settings(&app.state::<Arc<Core>>())
                    .map(|s| s.global_mark_shortcut)
                    .unwrap_or(false);
                if mark_global && !app.global_shortcut().is_registered(mark_shortcut()) {
                    let _ = app.global_shortcut().register(mark_shortcut());
                }
            }
            SessionState::Stopping
            | SessionState::Processing
            | SessionState::Ready
            | SessionState::Idle
            | SessionState::Failed => {
                let _ = app.global_shortcut().unregister(mark_shortcut());
                // The ticker shows Processing while a job runs.
                set_state(app, TrayState::Idle);
            }
            // A new session clears an old warning.
            SessionState::Starting => set_state(app, TrayState::Idle),
        },
        // Problems the user must act on. A failed write during a meeting is
        // shown in the window, not in the menu bar.
        Event::Error {
            kind: ErrorKind::Permission | ErrorKind::Capture,
            ..
        }
        | Event::Error {
            kind: ErrorKind::Storage,
            meeting: None,
            ..
        } => set_state(app, TrayState::Attention),
        _ => {}
    }
}

#[cfg(test)]
mod visibility_tests {
    use super::icon_visible;

    #[test]
    fn a_recording_keeps_the_icon_even_when_it_is_turned_off() {
        assert!(icon_visible(true, false));
        assert!(!icon_visible(false, false));
        assert!(icon_visible(false, true));
    }
}
