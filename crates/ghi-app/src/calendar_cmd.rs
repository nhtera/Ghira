// SPDX-License-Identifier: Apache-2.0
//! What a recorded meeting kept of its calendar event (sealed `calendar_ct`),
//! for both apps. Reading the calendar itself (EventKit, ICS) stays in each
//! app: events are never stored, only a recorded meeting keeps its own.

use ghi_core::calendar;
use serde::Serialize;
use specta::Type;

use crate::{CoreState, blocking};

/// The attendees of the calendar event a recorded meeting was named after
/// (rename suggestions list them first). Empty if none. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn meeting_attendees(
    core: CoreState<'_>,
    meeting: String,
) -> Result<Vec<String>, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        Ok(calendar::info(&store, &meeting)
            .map(|i| i.attendees)
            .unwrap_or_default())
    })
    .await
}

/// An attendee of the calendar event a meeting was recorded in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeetingContact {
    pub name: String,
    /// `None` when the invite had no address.
    pub email: Option<String>,
}

/// The attendees of the meeting's calendar event with their addresses (for
/// the follow-up email's "To"). Empty if none. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn meeting_contacts(
    core: CoreState<'_>,
    meeting: String,
) -> Result<Vec<MeetingContact>, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        Ok(calendar::info(&store, &meeting)
            .map(|i| {
                i.attendees
                    .into_iter()
                    .enumerate()
                    .map(|(n, name)| MeetingContact {
                        name,
                        email: i.emails.get(n).filter(|e| !e.is_empty()).cloned(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    })
    .await
}
