// SPDX-License-Identifier: Apache-2.0
//! Recording (M2). Contracts only: the bodies arrive with slice 16-D, which
//! drives them from `ghi-core` building blocks.

use ghi_core::events::SessionSnapshot;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::types::{ProcessingTarget, RecordPhase};
use ghi_app::system::MeetingLanguage;

/// v1 records in a room (the phone's microphone). Calls are not captured; M6
/// tells the user to use speakerphone and Room mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RecordMode {
    Room,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecordStart {
    pub mode: RecordMode,
    pub language: MeetingLanguage,
    /// A title, or `None` for the generated one.
    pub title: Option<String>,
    pub target: ProcessingTarget,
    /// The user agreed that everyone being recorded knows (M2 / M6 reminder).
    pub consent_acknowledged: bool,
}

/// The recording as it stands now, for a reloaded webview: apply `coreEvent`s
/// with a greater `seq` after `session`. Idle (nothing recorded): `phase` is
/// `idle` and `session` is `None`.
///
/// Two states exist and answer different questions. `phase` (and
/// `MobileEvent::Phase`) is the *iOS lifecycle* state of the recording: locked,
/// catching up, hot, interrupted, paused. `session.state` (from `coreEvent`) is
/// the *core* session state shared with the desktop (recording / paused /
/// stopping). The UI shows `phase`; it never infers one from the other.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecordState {
    pub phase: RecordPhase,
    /// Recorded seconds (not wall time: a pause does not count).
    pub elapsed_s: f64,
    pub marks: u32,
    /// Latest input level, dBFS (<= 0).
    pub level_db: f32,
    pub backlog_s: f64,
    pub catch_up_x: f64,
    pub pocket: bool,
    /// The live transcript is being made on this phone.
    pub live: bool,
    /// Why there is no live transcript, when there is none.
    pub record_only_reason: Option<RecordOnlyReason>,
    pub session: Option<SessionSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RecordOnlyReason {
    /// The device is below the live tier (iPhone 15 and RAM >= 6 GB).
    DeviceTier,
    ModelsMissing,
    Thermal,
    EngineFailed,
}

/// The copy-the-consent-message text (EN and VI) for the clipboard.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConsentMessage {
    pub en: String,
    pub vi: String,
}

/// What to ask after an interruption ended. `pending` is false when there is
/// nothing to ask (the other fields are then empty).
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResumePrompt {
    pub pending: bool,
    pub meeting: String,
    /// Seconds recorded before the interruption.
    pub recorded_s: f64,
    /// The interruption was a phone call.
    pub call: bool,
}

fn not_yet<T>() -> Result<T, String> {
    Err("not yet".into())
}

/// Starts a recording; returns the meeting id. Refused while a call is active
/// (`record_call_active`) until the user has acknowledged M6.
#[tauri::command]
#[specta::specta]
pub async fn record_start(_start: RecordStart) -> Result<String, String> {
    not_yet()
}

/// Stops and saves. Safe to call from the Live Activity while locked.
#[tauri::command]
#[specta::specta]
pub async fn record_stop() -> Result<(), String> {
    not_yet()
}

#[tauri::command]
#[specta::specta]
pub async fn record_pause() -> Result<(), String> {
    not_yet()
}

/// Resumes after a user pause or an interruption (restarts the audio engine).
#[tauri::command]
#[specta::specta]
pub async fn record_resume() -> Result<(), String> {
    not_yet()
}

/// Marks the current moment.
#[tauri::command]
#[specta::specta]
pub async fn record_mark() -> Result<(), String> {
    not_yet()
}

#[tauri::command]
#[specta::specta]
pub async fn record_snapshot() -> Result<RecordState, String> {
    not_yet()
}

#[tauri::command]
#[specta::specta]
pub async fn record_consent_message() -> Result<ConsentMessage, String> {
    not_yet()
}

/// A phone call is active right now (CXCallObserver).
#[tauri::command]
#[specta::specta]
pub async fn record_call_active() -> Result<bool, String> {
    not_yet()
}

/// The pending "Resume or stop and save?" question after an interruption.
#[tauri::command]
#[specta::specta]
pub async fn record_resume_prompt() -> Result<ResumePrompt, String> {
    not_yet()
}
