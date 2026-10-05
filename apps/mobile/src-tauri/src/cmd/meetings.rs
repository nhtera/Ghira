// SPDX-License-Identifier: Apache-2.0
//! Library status chips (M3). `list_meetings` (ghi-app) returns the shared
//! rows; the chip per row is derived on the Rust side by [`super::types::chip_for`]
//! so the UI never re-implements it.

use serde::{Deserialize, Serialize};
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
pub async fn meeting_chips(
    core: ghi_app::CoreState<'_>,
    ids: Vec<String>,
) -> Result<Vec<MeetingChipRow>, String> {
    ghi_app::blocking(&core, move |c| chips(c, &ids)).await
}

/// The file kinds the share sheet offers for a meeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ShareFormat {
    Md,
    Txt,
}

/// Renders the meeting (notes and transcript) as Markdown or text into a
/// temporary file under the data directory and presents the system share
/// sheet for it (the path never reaches the webview). Resolves once the sheet
/// is presented; Swift deletes the file when it closes, Rust after an hour or
/// at launch. Refused while the app is locked.
#[tauri::command]
#[specta::specta]
pub async fn share_meeting_export(
    core: ghi_app::CoreState<'_>,
    meeting: String,
    format: ShareFormat,
) -> Result<(), String> {
    ghi_app::blocking(&core, move |c| {
        let store = c.store()?;
        let vietnamese = ghi_app::system::load_settings(c)?.meeting_language
            == ghi_app::system::MeetingLanguage::Vi;
        let format = match format {
            ShareFormat::Md => ghi_core::export::Format::Markdown,
            ShareFormat::Txt => ghi_core::export::Format::Text,
        };
        crate::share::share_meeting(
            &store,
            c.data_dir(),
            &meeting,
            format,
            vietnamese,
            &crate::share::share_file,
        )
    })
    .await
}

/// The device tier is fixed for the life of the process: probed once.
pub(crate) fn device_is_live() -> bool {
    static LIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LIVE.get_or_init(|| crate::tier::detect().tier == super::lifecycle::TierClass::Live)
}

/// Reads the chosen meetings' rows (the library's own query) and maps them.
/// A final pass that cannot run here (the speech models are missing, or the
/// device is below the live tier) shows as waiting for models, not as progress.
pub fn chips(core: &ghi_app::core::Core, ids: &[String]) -> Result<Vec<MeetingChipRow>, String> {
    let can_process = crate::engine::engines_available(&core.models()) && device_is_live();
    let store = core.store()?;
    Ok(ghi_app::library::rows_by_gid(core, ids)?
        .iter()
        .map(|r| MeetingChipRow {
            gid: r.gid.clone(),
            chip: match super::types::chip_for(
                r,
                &ghi_app::sync_service::spoke::meeting_sync_view(&store, &r.gid),
            ) {
                MeetingChip::ProcessingOnPhone { .. } if !can_process => {
                    MeetingChip::WaitingForModels
                }
                chip => chip,
            },
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_app::core::{Core, CoreHooks};
    use ghi_store::keys::MemoryKeyStore;
    use ghi_store::store::NewMeeting;
    use std::sync::Arc;

    #[test]
    fn chips_follow_the_rows_and_wait_for_models_that_are_missing() {
        let t = tempfile::tempdir().unwrap();
        let (core, _rx) = Core::for_test_with(
            t.path().join("Ghira"),
            CoreHooks {
                handlers: Some(Arc::new(|_, _| vec![])),
                recover_kinds: Some(vec![]),
                keystore: Some(Arc::new(MemoryKeyStore::default())),
                ..CoreHooks::default()
            },
        );
        let store = core.store_even_locked().unwrap();
        let new = |title: &str| {
            store
                .create_meeting(NewMeeting {
                    title: title.into(),
                    ..Default::default()
                })
                .unwrap()
                .gid
        };
        let plain = new("recorded");
        let queued = new("queued");
        store
            .enqueue_job(Some(&queued), "final_pass", 1, &serde_json::json!({}))
            .unwrap();
        let ids = vec![queued.clone(), plain.clone(), "unknown".to_owned()];
        let rows = chips(&core, &ids).unwrap();
        // Unknown ids are left out; the order is the one asked for.
        let got: Vec<_> = rows
            .iter()
            .map(|r| (r.gid.as_str(), r.chip.clone()))
            .collect();
        assert_eq!(
            got,
            [
                (queued.as_str(), MeetingChip::WaitingForModels),
                (plain.as_str(), MeetingChip::Recorded),
            ]
        );
    }
}
