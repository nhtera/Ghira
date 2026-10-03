// SPDX-License-Identifier: Apache-2.0
//! Calendar (phase 14d): events from EventKit (macOS) or an ICS file, the "Up
//! next" strip, "ask to record when it starts", and naming a recording after
//! the event in progress. Events are never stored (D1); every command that
//! returns content refuses while the app is locked. Errors are codes.
//!
//! W0-B stub: the commands and types are final, the bodies are inert (slice
//! S5): `calendar_status` says the calendar is unavailable, `upcoming_events`
//! is empty, the rest answer `notImplemented`.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::core::Core;
use crate::{CoreState, blocking};

/// Not built yet (the W0-B stubs).
pub(crate) const NOT_IMPLEMENTED: &str = "notImplemented";

/// Calendar access and the connected ICS file.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    /// EventKit: `unavailable` (not macOS), `notDetermined`, `denied`,
    /// `authorized`.
    pub eventkit: String,
    /// The connected ICS file (its name only; the path stays in Rust).
    pub ics: Option<IcsInfo>,
    /// Ask to record when a calendar meeting starts.
    pub ask_on_start: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IcsInfo {
    pub name: String,
    /// Events found in the next 14 days.
    pub events: u32,
}

/// Calendar settings the user can change.
#[derive(Debug, Clone, Deserialize, Type)]
#[allow(dead_code)] // read once slice S5 fills in `set_calendar`
#[serde(rename_all = "camelCase")]
pub struct CalendarPatch {
    #[serde(default)]
    #[specta(optional)]
    pub ask_on_start: Option<bool>,
}

/// One upcoming event, as the Up next strip and the popover show it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EventView {
    /// Stable per occurrence; pass it to `set_event_ask`.
    pub key: String,
    pub title: String,
    pub start_ms: f64,
    pub end_ms: f64,
    /// Other attendees (a count; names only inside a recorded meeting).
    pub attendees: u32,
    /// `zoom`, `teams`, `meet` or `null`.
    pub join_app: Option<String>,
    /// Ask to record when it starts (armed by default for meeting-like events).
    pub ask: bool,
}

fn inert_status() -> CalendarStatus {
    CalendarStatus {
        eventkit: "unavailable".into(),
        ics: None,
        ask_on_start: false,
    }
}

/// Where calendar access stands. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn calendar_status(core: CoreState<'_>) -> Result<CalendarStatus, String> {
    blocking(&core, |c| {
        c.store()?;
        Ok(inert_status())
    })
    .await
}

/// Asks macOS for calendar access (the OS prompt). Errors: `notImplemented`.
#[tauri::command]
#[specta::specta]
pub async fn request_calendar_access(core: CoreState<'_>) -> Result<CalendarStatus, String> {
    blocking(&core, |_| Err(NOT_IMPLEMENTED.into())).await
}

/// Changes calendar settings; returns the new status. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn set_calendar(
    core: CoreState<'_>,
    patch: CalendarPatch,
) -> Result<CalendarStatus, String> {
    blocking(&core, move |_| {
        let _ = patch;
        Err(NOT_IMPLEMENTED.into())
    })
    .await
}

/// Lets the user pick an .ics file (a native dialog in Rust); returns its
/// name, or `null` if cancelled. Errors: `icsInvalid`, `notImplemented`.
#[tauri::command]
#[specta::specta]
pub async fn pick_ics_file(core: CoreState<'_>) -> Result<Option<String>, String> {
    blocking(&core, |_| Err(NOT_IMPLEMENTED.into())).await
}

/// Forgets the ICS file. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn remove_ics_file(core: CoreState<'_>) -> Result<(), String> {
    blocking(&core, |_| Err(NOT_IMPLEMENTED.into())).await
}

/// The next events (at most `limit`, soonest first), meeting-like ones
/// included whether armed or not. Empty without a calendar. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn upcoming_events(core: CoreState<'_>, limit: u32) -> Result<Vec<EventView>, String> {
    blocking(&core, move |c| {
        c.store()?;
        let _ = limit;
        Ok(Vec::new())
    })
    .await
}

/// Turns "ask to record when it starts" on or off for one event. Errors:
/// `storage`.
#[tauri::command]
#[specta::specta]
pub async fn set_event_ask(core: CoreState<'_>, key: String, ask: bool) -> Result<(), String> {
    blocking(&core, move |_| {
        let _ = (key, ask);
        Err(NOT_IMPLEMENTED.into())
    })
    .await
}

/// The attendees of the calendar event a recorded meeting was named after
/// (rename suggestions list them first). Empty if none. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn meeting_attendees(
    core: CoreState<'_>,
    meeting: String,
) -> Result<Vec<String>, String> {
    blocking(&core, move |c| {
        c.store()?;
        let _ = meeting;
        Ok(Vec::new())
    })
    .await
}

/// Called after a recording started: a meeting-like event in progress (from
/// 10 minutes before its start to its end), or the one the user was just
/// prompted for, gives the meeting its title (when it has none) and its
/// sealed calendar info (D4). Never fails the recording.
pub(crate) fn on_recording_started(_core: &Core, _meeting: &str) {}
