// SPDX-License-Identifier: Apache-2.0
//! LAN sync between the desktop and a phone (phase 15, doc 07): the commands
//! and DTOs both apps' UI use. The commands run on the app's
//! [`SyncService`] (managed state, see `sync_service.rs`); an app without one
//! (the phone, until its wiring lands) gets an honest empty state (sync off,
//! nobody paired) or the typed error [`NOT_AVAILABLE`].
//!
//! The desktop registers the hub side (`sync_pair_open`/`sync_pair_close`);
//! the phone registers the spoke side (`sync_pair_scan_*`, `sync_lease_revoke`).
//! Everything else is common. Events and errors carry codes only, never
//! meeting content, names of meetings or keys (doc 07 §5.3 `Error{code}`).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri_specta::Event;

use crate::sync_service::SyncService;

/// The error a command returns while its part of sync isn't wired in yet.
pub const NOT_AVAILABLE: &str = "sync_not_available";

/// How long a pairing code stays valid (doc 07 §3.2), in ms.
pub const PAIR_CODE_TTL_MS: f64 = 120_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DevicePlatform {
    Mac,
    Windows,
    Ios,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DeviceState {
    /// Paired and syncing.
    Paired,
    /// "Unpair and wipe" was chosen; waiting for the device to be reachable
    /// so it can delete what it holds from this one (doc 07 §3.5).
    WipePending,
}

/// One paired device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRow {
    pub gid: String,
    pub name: String,
    pub platform: DevicePlatform,
    pub state: DeviceState,
    /// Unix ms of the last completed session; `None` if never.
    pub last_seen_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub enabled: bool,
    pub paired: Vec<DeviceRow>,
    /// Things waiting to go to the desktop; the desktop shows "Open Ghira on
    /// your phone to sync" while this is above 0.
    pub pending_on_phone: u32,
    /// Always true: sync never leaves the local network.
    pub local_only: bool,
    /// The last failure as a code (never content), for the UI to word.
    pub last_error_code: Option<String>,
}

impl Default for SyncStatus {
    fn default() -> Self {
        SyncStatus {
            enabled: false,
            paired: Vec::new(),
            pending_on_phone: 0,
            local_only: true,
            last_error_code: None,
        }
    }
}

/// A pairing code to show on the desktop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PairOffer {
    /// The QR code as an SVG document (rendered in Rust; the webview only displays it).
    pub qr_svg: String,
    /// Ms until the code stops working ([`PAIR_CODE_TTL_MS`]).
    pub expires_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ConflictTarget {
    Title,
    Segment,
    NoteBlock,
    ActionItem,
    Speaker,
}

/// The losing side of an edit made on two devices: kept so the user can take
/// it instead (doc 07 §7.4). The winner is already in place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConflictCopy {
    pub gid: String,
    pub target_kind: ConflictTarget,
    /// Which field of the target (`text`, `title`, ...).
    pub field: String,
    /// The name of the device that made this edit ("Edited on {device}").
    pub device: String,
    pub text: String,
}

/// Where "Delete everywhere" stands (doc 07 §7.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DeleteEverywhereState {
    Idle,
    /// Waiting for the devices in `waiting_for` to be reachable.
    Waiting,
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeleteEverywhereStatus {
    pub state: DeleteEverywhereState,
    /// Names of the devices not yet reached.
    pub waiting_for: Vec<String>,
}

/// A sync failure as a code the UI words (doc 07 §5.3: never content).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SyncErrorCode {
    /// No network path to the other device on the local network.
    Unreachable,
    /// The other device no longer accepts this one.
    Refused,
    /// The two apps are too far apart in version; one needs an update.
    UpgradeRequired,
    StorageFull,
    /// Locked by the app lock.
    Locked,
    Internal,
}

/// Sync happened (or needs the user). Codes and counts only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Event)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SyncEvent {
    Paired {
        device: DeviceRow,
    },
    /// `by_peer`: the other device unpaired (the UI says "Unpaired by <name>").
    Unpaired {
        gid: String,
        name: String,
        by_peer: bool,
    },
    Progress {
        pending: u32,
    },
    /// A conflict copy was made for this meeting.
    Conflict {
        meeting: String,
    },
    /// The other device deleted `count` meetings in one go; nothing is
    /// applied until the user answers `sync_confirm_mass_delete`.
    NeedsConfirm {
        device: String,
        count: u32,
    },
    /// The wipe finished (here, or the peer confirmed it).
    WipeDone {
        gid: String,
    },
    Error {
        code: SyncErrorCode,
    },
}

/// The service behind the commands, when the app runs one (the desktop does;
/// the phone's wiring is 15-J2, until then its commands answer the empty
/// state or [`NOT_AVAILABLE`]).
fn service(app: &tauri::AppHandle) -> Option<Arc<SyncService>> {
    use tauri::Manager;
    app.try_state::<Arc<SyncService>>()
        .map(|s| s.inner().clone())
}

/// Runs `f` on the service off the async runtime's workers, or `without`
/// when the app has no service.
async fn with_service<T: Send + 'static>(
    app: &tauri::AppHandle,
    f: impl FnOnce(Arc<SyncService>) -> Result<T, String> + Send + 'static,
    without: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    match service(app) {
        Some(s) => tauri::async_runtime::spawn_blocking(move || f(s))
            .await
            .map_err(|e| e.to_string())?,
        None => without(),
    }
}

/// Whether sync is on, who is paired and what is waiting.
#[tauri::command]
#[specta::specta]
pub async fn sync_status(app: tauri::AppHandle) -> Result<SyncStatus, String> {
    with_service(&app, |s| s.status(), || Ok(SyncStatus::default())).await
}

/// Turns sync on or off. Turning it on creates this device's identity.
#[tauri::command]
#[specta::specta]
pub async fn sync_set_enabled(app: tauri::AppHandle, enabled: bool) -> Result<SyncStatus, String> {
    with_service(
        &app,
        move |s| s.set_enabled(enabled),
        move || {
            if enabled {
                Err(NOT_AVAILABLE.into())
            } else {
                Ok(SyncStatus::default())
            }
        },
    )
    .await
}

/// Desktop: opens the pairing window and returns the QR code to show.
#[tauri::command]
#[specta::specta]
pub async fn sync_pair_open(app: tauri::AppHandle) -> Result<PairOffer, String> {
    with_service(&app, |s| s.pair_open(), || Err(NOT_AVAILABLE.into())).await
}

/// Desktop: closes the pairing window (the code stops working).
#[tauri::command]
#[specta::specta]
pub async fn sync_pair_close(app: tauri::AppHandle) -> Result<(), String> {
    with_service(&app, |s| s.pair_close(), || Ok(())).await
}

#[tauri::command]
#[specta::specta]
pub async fn sync_devices(app: tauri::AppHandle) -> Result<Vec<DeviceRow>, String> {
    with_service(&app, |s| s.devices(), || Ok(Vec::new())).await
}

/// Forgets a device. Meetings already synced stay on both.
#[tauri::command]
#[specta::specta]
pub async fn sync_unpair(app: tauri::AppHandle, gid: String) -> Result<(), String> {
    with_service(&app, move |s| s.unpair(&gid), || Err(NOT_AVAILABLE.into())).await
}

/// Forgets a device and has it delete what it got from this one.
#[tauri::command]
#[specta::specta]
pub async fn sync_unpair_and_wipe(app: tauri::AppHandle, gid: String) -> Result<(), String> {
    with_service(
        &app,
        move |s| s.unpair_and_wipe(&gid),
        || Err(NOT_AVAILABLE.into()),
    )
    .await
}

/// Syncs now instead of waiting for the next change.
#[tauri::command]
#[specta::specta]
pub async fn sync_now(app: tauri::AppHandle) -> Result<(), String> {
    with_service(&app, |s| s.now(), || Err(NOT_AVAILABLE.into())).await
}

/// The conflict copies of a meeting.
#[tauri::command]
#[specta::specta]
pub async fn sync_conflicts(
    app: tauri::AppHandle,
    meeting: String,
) -> Result<Vec<ConflictCopy>, String> {
    with_service(&app, move |s| s.conflicts(&meeting), || Ok(Vec::new())).await
}

/// Takes the copy (`use_it`) in place of what is there, or dismisses it.
#[tauri::command]
#[specta::specta]
pub async fn sync_conflict_resolve(
    app: tauri::AppHandle,
    gid: String,
    use_it: bool,
) -> Result<(), String> {
    with_service(
        &app,
        move |s| s.conflict_resolve(&gid, use_it),
        || Err(NOT_AVAILABLE.into()),
    )
    .await
}

/// Answers a [`SyncEvent::NeedsConfirm`]: apply the other device's mass delete or refuse it.
#[tauri::command]
#[specta::specta]
pub async fn sync_confirm_mass_delete(app: tauri::AppHandle, accept: bool) -> Result<(), String> {
    with_service(
        &app,
        move |s| s.confirm_mass_delete(accept),
        || Err(NOT_AVAILABLE.into()),
    )
    .await
}

/// Where "Delete everything" stands: waiting for paired devices to take their
/// wipe, or finished.
#[tauri::command]
#[specta::specta]
pub async fn sync_delete_everywhere_status(
    app: tauri::AppHandle,
) -> Result<DeleteEverywhereStatus, String> {
    match service(&app) {
        Some(s) => Ok(s.delete_everywhere_status()),
        None => Ok(DeleteEverywhereStatus {
            state: DeleteEverywhereState::Idle,
            waiting_for: Vec::new(),
        }),
    }
}

/// "Delete here only": stops the wait for paired devices during "Delete
/// everything"; the data goes at once and the devices that were not reached
/// keep their copies.
#[tauri::command]
#[specta::specta]
pub async fn sync_delete_everywhere_skip(app: tauri::AppHandle) -> Result<(), String> {
    match service(&app) {
        Some(s) => {
            s.delete_everywhere_skip();
            Ok(())
        }
        None => Err(NOT_AVAILABLE.into()),
    }
}

/// Phone: starts the camera to scan the desktop's code (the scan arrives as
/// `ghi_ios_qr_scanned`, never through the webview).
#[tauri::command]
#[specta::specta]
pub async fn sync_pair_scan_start() -> Result<(), String> {
    Err(NOT_AVAILABLE.into())
}

#[tauri::command]
#[specta::specta]
pub async fn sync_pair_scan_stop() -> Result<(), String> {
    Ok(())
}

/// Phone: takes back a meeting's final pass from the desktop to run it here
/// ("Process on this phone now").
#[tauri::command]
#[specta::specta]
pub async fn sync_lease_revoke(meeting: String) -> Result<(), String> {
    let _ = meeting;
    Err(NOT_AVAILABLE.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_state_is_off_local_and_unpaired() {
        let s = SyncStatus::default();
        assert!(!s.enabled && s.local_only && s.paired.is_empty());
        assert_eq!(s.pending_on_phone, 0);
    }

    #[test]
    fn events_carry_a_type_tag_and_no_free_text_errors() {
        let json = serde_json::to_value(SyncEvent::Error {
            code: SyncErrorCode::UpgradeRequired,
        })
        .unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["code"], "upgradeRequired");
    }
}
