// SPDX-License-Identifier: Apache-2.0
//! `MobileEvent`: what the iOS shell tells the webview, beside the shared
//! `coreEvent` (transcript, speakers, jobs). Swift reaches it through the
//! `ghi_ios_*` exports in `platform.rs`; Rust emits it with [`emit`].

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;
use tauri_specta::Event;

use super::models::MobileModelItem;
use super::types::RecordPhase;

/// Why the audio session was interrupted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum InterruptionKind {
    /// A phone call (the M6 notice).
    Call,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MobileEvent {
    /// The recording phase changed.
    Phase { phase: RecordPhase },
    /// Audio recorded while locked or hot that the transcript has not caught up with.
    Backlog { backlog_s: f64, catch_up_x: f64 },
    /// The microphone sounds muffled (a pocket or bag): warn, clear when it passes.
    Pocket { muffled: bool },
    /// The audio session was interrupted (began) or the interruption ended.
    /// Recording never resumes by itself: the UI asks (`record_resume_prompt`).
    Interruption { began: bool, kind: InterruptionKind },
    /// `ProcessInfo.thermalState` raw value, 0 nominal to 3 critical.
    Thermal { level: u8 },
    /// Dynamic Type as a multiplier, capped at 2.0 (the CSS variable `--ghi-text-scale`).
    TextScale { scale: f32 },
    /// The share extension added files to the App Group inbox.
    InboxChanged,
    /// The audio route changed (`AVAudioSession.RouteChangeReason` raw value).
    Route { reason: i32 },
    /// The system warned about memory.
    MemoryWarning,
    /// A model download moved on.
    ModelDownload { item: MobileModelItem },
    /// A phone call started or ended (the M6 notice blocks Record while active).
    CallActive { active: bool },
}

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Remembers the app handle so Swift callbacks can emit events.
pub fn install(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

/// Emits to the webview; dropped before [`install`] or when the webview is gone.
pub fn emit(event: MobileEvent) {
    if let Some(app) = APP.get() {
        let _ = event.emit(app);
    }
}
