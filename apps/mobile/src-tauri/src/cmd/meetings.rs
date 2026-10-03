// SPDX-License-Identifier: Apache-2.0
//! Library status chips (M3). `list_meetings` (ghi-app) returns the shared
//! rows; the chip per row is derived on the Rust side by [`super::types::chip_for`]
//! so the UI never re-implements it.

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
pub async fn meeting_chips(
    core: ghi_app::CoreState<'_>,
    ids: Vec<String>,
) -> Result<Vec<MeetingChipRow>, String> {
    ghi_app::blocking(&core, move |c| chips(c, &ids)).await
}

/// The device tier is fixed for the life of the process: probed once.
fn device_is_live() -> bool {
    static LIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LIVE.get_or_init(|| crate::tier::detect().tier == super::lifecycle::TierClass::Live)
}

/// Reads the chosen meetings' rows (the library's own query) and maps them.
/// A final pass that cannot run here (the speech models are missing, or the
/// device is below the live tier) shows as waiting for models, not as progress.
pub fn chips(core: &ghi_app::core::Core, ids: &[String]) -> Result<Vec<MeetingChipRow>, String> {
    let can_process = crate::engine::engines_available(&core.models()) && device_is_live();
    Ok(ghi_app::library::rows_by_gid(core, ids)?
        .iter()
        .map(|r| MeetingChipRow {
            gid: r.gid.clone(),
            chip: match super::types::chip_for(r) {
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
