// SPDX-License-Identifier: Apache-2.0
//! Recording (M2), driven by the [`Recorder`] (`session.rs`) over `ghi-core`
//! building blocks. The recorder is Tauri state the mobile core manages
//! (`app.manage(Arc<Recorder>)`); a command before that answers `not ready`.
//!
//! ## Lifecycle contract for the shell (16-E / 16-G)
//!
//! See `lifecycle.rs` for the table. In short: `willResignActive` closes the
//! engine gate, `didEnterBackground` also calls `JobRunner::app_inactive`,
//! `didBecomeActive` opens the gate and calls `app_active`; uninterruptible
//! native calls (diarizer `finish`, a 10 s block push) run inside the engine
//! gate; a launch in the background syncs the runner before `spawn`.
//!
//! ## What the UI can rely on
//!
//! - `record_start` is refused while a call is active (error `call_active`)
//!   until the user acknowledged M6 (`call_acknowledged`), with
//!   `microphone_denied`, with `disk_low` (< 500 MB free), for the `Desktop`
//!   target, and for the `Phone` target on a device below the live tier. A
//!   previous recording still being transcribed is waited for (up to a minute).
//! - A below-tier device records only (`DeviceTier`): no job is queued, the
//!   meeting stays `done` (recorded; the chip is "Recorded") and the session
//!   goes `idle`, never `ready`. Missing models also record only
//!   (`ModelsMissing`), but the final pass is queued and waits for them.
//! - An interruption pauses the recording (`MobileEvent::Interruption`);
//!   when it ends `record_resume_prompt` answers `pending`. Recording never
//!   resumes by itself: `record_resume` restarts the audio engine.
//! - The meeting is `processing` with a `final_pass` job once the engine has
//!   drained; a final pass on the phone queues no notes, so the meeting
//!   becomes `ready` (with `StateChanged { ready }`) when the pass ends.
//! - The phone registers `VoiceLearnJob` whenever Me is enrolled
//!   (`engine::job_handlers`); the voice step queues `voice_learn`.

use std::sync::Arc;

use ghi_core::events::SessionSnapshot;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::Manager;

use super::types::{ProcessingTarget, RecordPhase};
use crate::platform;
use crate::session::Recorder;
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
    /// The user agreed that everyone being recorded knows (M2 reminder).
    pub consent_acknowledged: bool,
    /// The user saw the M6 notice (use speakerphone and Room mode) while a
    /// phone call is active. Only this lifts the call block; the consent flag
    /// above never does.
    pub call_acknowledged: bool,
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

fn recorder(app: &tauri::AppHandle) -> Result<Arc<Recorder>, String> {
    app.try_state::<Arc<Recorder>>()
        .map(|r| r.inner().clone())
        .ok_or_else(|| "not ready".to_owned())
}

/// Runs blocking recorder work off the async runtime's threads.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

/// Starts a recording; returns the meeting id. Refused while a call is active
/// (`call_active`) until the user has acknowledged M6.
#[tauri::command]
#[specta::specta]
pub async fn record_start(app: tauri::AppHandle, start: RecordStart) -> Result<String, String> {
    let rec = recorder(&app)?;
    blocking(move || {
        // A "delete everything" cannot begin while a start is under way.
        let _guard = crate::privacy_cmd::DATA_GUARD
            .read()
            .unwrap_or_else(|e| e.into_inner());
        rec.start(&start)
    })
    .await
}

/// Stops and saves. Safe to call from the Live Activity while locked.
#[tauri::command]
#[specta::specta]
pub async fn record_stop(app: tauri::AppHandle) -> Result<(), String> {
    let rec = recorder(&app)?;
    blocking(move || rec.stop()).await
}

#[tauri::command]
#[specta::specta]
pub async fn record_pause(app: tauri::AppHandle) -> Result<(), String> {
    recorder(&app)?.pause()
}

/// Resumes after a user pause or an interruption (restarts the audio engine).
#[tauri::command]
#[specta::specta]
pub async fn record_resume(app: tauri::AppHandle) -> Result<(), String> {
    let rec = recorder(&app)?;
    blocking(move || rec.resume()).await
}

/// Marks the current moment.
#[tauri::command]
#[specta::specta]
pub async fn record_mark(app: tauri::AppHandle) -> Result<(), String> {
    recorder(&app)?.mark()
}

#[tauri::command]
#[specta::specta]
pub async fn record_snapshot(app: tauri::AppHandle) -> Result<RecordState, String> {
    let rec = recorder(&app)?;
    blocking(move || Ok(rec.snapshot())).await
}

#[tauri::command]
#[specta::specta]
pub async fn record_consent_message() -> Result<ConsentMessage, String> {
    Ok(Recorder::consent_message())
}

/// A phone call is active right now (CXCallObserver).
#[tauri::command]
#[specta::specta]
pub async fn record_call_active() -> Result<bool, String> {
    Ok(platform::call_active())
}

/// The pending "Resume or stop and save?" question after an interruption.
#[tauri::command]
#[specta::specta]
pub async fn record_resume_prompt(app: tauri::AppHandle) -> Result<ResumePrompt, String> {
    Ok(recorder(&app)?.resume_prompt())
}
