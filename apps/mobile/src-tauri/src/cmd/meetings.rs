// SPDX-License-Identifier: Apache-2.0
//! Library status chips (M3). `list_meetings` (ghi-app) returns the shared
//! rows; the chip per row is derived on the Rust side by [`super::types::chip_for`]
//! so the UI never re-implements it. Contracts only: the body arrives with 16-I.

use serde::Serialize;
use specta::Type;

use super::types::MeetingChip;

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeetingChipRow {
    pub gid: String,
    pub chip: MeetingChip,
}

/// The chip for each of these meetings (unknown ids are left out). Call it
/// with the ids `list_meetings` returned and again on `coreEvent` job changes.
#[tauri::command]
#[specta::specta]
pub async fn meeting_chips(_ids: Vec<String>) -> Result<Vec<MeetingChipRow>, String> {
    Err("not yet".into())
}
