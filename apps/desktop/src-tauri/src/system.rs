// SPDX-License-Identifier: Apache-2.0
//! OS permissions and meeting detection (the settings themselves are in
//! `ghi-app`).
//!
//! - Microphone permission comes from AVFoundation; system-audio access can't
//!   be queried on macOS (a denied tap just delivers silence, which the core
//!   reports as `silentSystemTrack`).
//! - The detection poller lists the processes using audio every 2 s while
//!   nothing is recording and emits `MeetingDetected` (the UI shows its own
//!   non-activating prompt). Never/snooze choices persist under `detect`.

use std::sync::{Arc, Mutex};
#[cfg(target_os = "macos")]
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;
use tauri::Manager;
use tauri_specta::Event;

pub use ghi_app::system::*;

use crate::core::Core;
use crate::{CoreState, blocking};

const DETECT_KEY: &str = "detect";

/// Microphone access as macOS reports it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    Granted,
    Denied,
    /// Not asked yet.
    Undetermined,
    /// Blocked by a policy (MDM, parental controls).
    Restricted,
    /// Not applicable on this platform (no macOS permission model).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Unsupported,
}

#[cfg(target_os = "macos")]
fn from_mac(p: ghi_audio::macos::MicPermission) -> Permission {
    use ghi_audio::macos::MicPermission as M;
    match p {
        M::Authorized => Permission::Granted,
        M::Denied => Permission::Denied,
        M::Undetermined => Permission::Undetermined,
        M::Restricted => Permission::Restricted,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn mic_permission() -> Permission {
    #[cfg(target_os = "macos")]
    return from_mac(ghi_audio::macos::mic_permission());
    #[cfg(not(target_os = "macos"))]
    Permission::Unsupported
}

/// Shows the OS prompt when undetermined (blocks until answered).
#[tauri::command]
#[specta::specta]
pub async fn request_mic_permission() -> Result<Permission, String> {
    #[cfg(target_os = "macos")]
    return tauri::async_runtime::spawn_blocking(|| {
        from_mac(ghi_audio::macos::request_mic_permission())
    })
    .await
    .map_err(|e| e.to_string());
    #[cfg(not(target_os = "macos"))]
    Ok(Permission::Unsupported)
}

/// A pane of the OS privacy settings, for a denied permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PrivacyPane {
    Microphone,
    /// "Screen & System Audio Recording" on macOS.
    SystemAudio,
    Notifications,
    /// "Calendars" on macOS.
    Calendars,
}

/// Opens System Settings on the pane where the user can turn access on.
#[tauri::command]
#[specta::specta]
pub fn open_privacy_settings(pane: PrivacyPane) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Fixed URLs only: nothing from the webview reaches the command line.
        let url = match pane {
            PrivacyPane::Microphone => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
            PrivacyPane::SystemAudio => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
            PrivacyPane::Notifications => {
                "x-apple.systempreferences:com.apple.Notifications-Settings.extension"
            }
            PrivacyPane::Calendars => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars"
            }
        };
        // Started, not awaited: a command must not block on another process.
        std::process::Command::new("/usr/bin/open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pane;
        Err("not available on this platform yet".into())
    }
}

/// A meeting app started using the microphone.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetected {
    /// Stable app key (`zoom`, `teams`, `chrome`, …), for the reply.
    pub app: String,
    /// Shown in the prompt ("Zoom"); browsers prompt generically.
    pub app_name: String,
    pub browser: bool,
    /// The calendar event this meeting is (its title), when there is one.
    #[serde(default)]
    #[specta(optional)]
    pub title: Option<String>,
    /// That event's key (for the reply and the dedupe with the calendar ticker).
    #[serde(default)]
    #[specta(optional)]
    pub event: Option<String>,
}

/// The user's answer to a detection prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DetectReply {
    /// Recording starts (the UI calls start_recording itself).
    Start,
    /// Quiet for this call (the 30 min cooldown already applies).
    NotNow,
    /// Never ask for this app again.
    Never,
}

/// The detector, shared by the poller and the reply command.
#[derive(Default)]
pub struct Detection(Mutex<Option<ghi_audio::detect::Detector>>);

fn persist(core: &Core, d: &ghi_audio::detect::Detector) {
    if let (Ok(store), Ok(v)) = (core.store(), serde_json::to_value(d.state())) {
        let _ = store.set_setting(DETECT_KEY, &v);
    }
}

#[tauri::command]
#[specta::specta]
pub async fn reply_meeting_detected(
    core: CoreState<'_>,
    detection: tauri::State<'_, Arc<Detection>>,
    app: String,
    reply: DetectReply,
) -> Result<(), String> {
    // Anything but Start forgets the calendar event the prompt was about.
    crate::calendar_cmd::prompt_answered(reply == DetectReply::Start);
    // A calendar-only prompt has no app to remember (and no "Never").
    if app == crate::calendar_cmd::CALENDAR_APP {
        return Ok(());
    }
    let Some(app) = ghi_audio::detect::App::from_key(&app) else {
        return Err("unknown app".into());
    };
    let detection = detection.inner().clone();
    blocking(&core, move |c| {
        let mut slot = detection.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(d) = slot.as_mut() {
            match reply {
                DetectReply::Never => d.never(app),
                DetectReply::NotNow | DetectReply::Start => {}
            }
            persist(c, d);
        }
        Ok(())
    })
    .await
}

/// Starts the detection poller (macOS; elsewhere a no-op until phase 13).
pub fn spawn_detection(app: AppHandle, core: Arc<Core>, detection: Arc<Detection>) {
    // Calendar meetings (EventKit on macOS, an ICS file anywhere).
    crate::calendar_cmd::spawn_ticker(app.clone(), core.clone());
    #[cfg(target_os = "macos")]
    {
        let _ = std::thread::Builder::new()
            .name("ghi-detect".into())
            .spawn(move || detection_loop(app, core, detection));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, core, detection);
}

#[cfg(target_os = "macos")]
fn detection_loop(app: AppHandle, core: Arc<Core>, detection: Arc<Detection>) {
    use ghi_audio::detect::{DetectConfig, DetectState, Detector};
    // The store opens in the background at launch; detection starts after
    // (and keeps retrying if it can't open yet, e.g. a locked keychain).
    let store = loop {
        match core.store() {
            Ok(s) => break s,
            Err(_) => std::thread::sleep(Duration::from_secs(30)),
        }
    };
    let state: DetectState = store
        .get_setting(DETECT_KEY)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    *detection.0.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Detector::new(DetectConfig::default(), state));
    loop {
        std::thread::sleep(Duration::from_secs(2));
        // Starting or recording: no prompt (and no store reads meanwhile).
        if core.busy()
            || !load_settings(&core)
                .map(|s| s.detect_meetings)
                .unwrap_or(false)
        {
            continue;
        }
        let Ok(processes) = ghi_audio::macos::audio_processes() else {
            continue;
        };
        let prompt = {
            let mut slot = detection.0.lock().unwrap_or_else(|e| e.into_inner());
            let Some(d) = slot.as_mut() else { continue };
            let p = d.poll(&processes, SystemTime::now());
            if p.is_some() {
                persist(&core, d);
            }
            p
        };
        if let Some(p) = prompt {
            // Inside a calendar meeting the prompt carries its title; one
            // already asked about (by the calendar) is not asked again (D3).
            let (title, event) = match crate::calendar_cmd::for_detection(&core) {
                Some(crate::calendar_cmd::Attach::Skip) => continue,
                Some(crate::calendar_cmd::Attach::Event { title, key }) => (Some(title), Some(key)),
                None => (None, None),
            };
            show_detect(
                &app,
                MeetingDetected {
                    app: p.app.key().into(),
                    app_name: p.app.display_name().into(),
                    browser: p.app.is_browser(),
                    title,
                    event,
                },
            );
        }
    }
}

/// Shows a detection prompt: in the main window only when the user is looking
/// at it; else (hidden, behind a full-screen call, another Space) the panel.
pub fn show_detect(app: &AppHandle, detected: MeetingDetected) {
    let main_focused = app
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    if main_focused {
        let _ = detected.emit_to(app, "main");
    } else {
        crate::panels::open_detect(app, detected);
    }
}

/// A system notification (notes ready, recovered); clicking it brings the
/// app forward. The text comes localized from the UI.
#[tauri::command]
#[specta::specta]
pub fn show_notification(app: tauri::AppHandle, title: String, body: String) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    let clip = |s: String, n: usize| s.chars().take(n).collect::<String>();
    app.notification()
        .builder()
        .title(clip(title, 120))
        .body(clip(body, 400))
        .show()
        .map_err(|e| e.to_string())
}
