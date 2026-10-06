// SPDX-License-Identifier: Apache-2.0
//! Types shared by the mobile commands and events (phase 16 contracts).

use ghi_app::sync_service::spoke::{GrantorState, SyncView};
use serde::{Deserialize, Serialize};
use specta::Type;

/// Where a recording is processed after it stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ProcessingTarget {
    /// The final pass runs on this phone (needs the speech models and a capable device).
    Phone,
    /// Phase 15: a paired desktop (it needs a pairing; the recording is
    /// handed over under a lease once its audio is there).
    Desktop,
    /// Notes through the user's own cloud key, transcript text only, per
    /// meeting and after a preview. The transcript itself is still made on the phone.
    Cloud,
}

/// The status chip on a meeting row (M3). The last three are phase 15 and
/// only exist while a computer is paired.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MeetingChip {
    Recorded,
    ProcessingOnPhone {
        percent: u8,
    },
    ProcessedOnPhone,
    WaitingForModels,
    Failed,
    // Phase 15.
    Synced,
    WaitingForWifi,
    /// The computer had the pass and its lease ran out; offered again when it is back.
    WaitingForComputer,
    FinalOnDesktop {
        percent: u8,
    },
}

/// What the session is doing, for the UI and the Live Activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RecordPhase {
    Idle,
    /// Loading the models (first recording, or after a reload).
    Loading,
    Live,
    /// The app is in the background: recording only; the transcript catches up later.
    Locked,
    CatchingUp,
    /// Thermal state serious or worse: recording only.
    Hot,
    /// A phone call or another app took the audio session; waiting for the user to resume.
    Interrupted,
    /// Paused by the user.
    Paused,
    /// Stopped; the engine finishes the backlog.
    Finishing,
    Done,
    /// No live engine (device below tier, models missing or failed): recording only.
    RecordOnly,
}

/// The status chip for a library row (M3). `MeetingChipRow` carries it to the
/// UI (`meeting_chips`), so 16-I has one per meeting without re-deriving it.
/// `sync` is where the meeting stands with the paired computer (empty without
/// one): a lease open on the computer is "Final pass on <device> · %", a
/// meeting that has not reached it (or whose computer is away) waits for
/// Wi-Fi, and a processed meeting the computer has is synced.
pub fn chip_for(row: &ghi_app::library::MeetingRow, sync: &SyncView) -> MeetingChip {
    if let Some(l) = sync.lease {
        match l.state {
            GrantorState::Granted | GrantorState::Revoking => {
                return MeetingChip::FinalOnDesktop { percent: l.percent };
            }
            GrantorState::Offered => return MeetingChip::WaitingForWifi,
            GrantorState::Expired => return MeetingChip::WaitingForComputer,
            GrantorState::Done | GrantorState::SelfTaken => {}
        }
    }
    // Recorded for the computer and not handed over yet.
    if sync.pending && row.job.is_none() {
        return MeetingChip::WaitingForWifi;
    }
    if let Some(job) = &row.job {
        if job.waiting_for_models {
            return MeetingChip::WaitingForModels;
        }
        // 0..1 progress of the active job (final pass on the phone).
        let percent = (job.progress.clamp(0.0, 1.0) * 100.0).round() as u8;
        return MeetingChip::ProcessingOnPhone { percent };
    }
    match row.status.as_str() {
        "failed" => MeetingChip::Failed,
        "ready" if sync.synced => MeetingChip::Synced,
        "ready" => MeetingChip::ProcessedOnPhone,
        _ => MeetingChip::Recorded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_app::library::{MeetingJob, MeetingRow};

    fn row(status: &str, job: Option<MeetingJob>) -> MeetingRow {
        MeetingRow {
            gid: "g".into(),
            title: "t".into(),
            started_at: 0.0,
            duration_ms: 0.0,
            source: "live".into(),
            mode: "room".into(),
            status: status.into(),
            transcript_version: 1.0,
            cloud_used: false,
            consent_confirmed: false,
            sensitive: false,
            template: None,
            people: vec![],
            job,
            folder: None,
            tags: vec![],
            source_app: None,
            summary: None,
            unnamed_voices: 0,
        }
    }

    fn job(progress: f64, waiting: bool) -> Option<MeetingJob> {
        Some(MeetingJob {
            kind: "final_pass".into(),
            progress,
            waiting_for_models: waiting,
        })
    }

    use ghi_app::sync_service::spoke::LeaseView;

    fn lease(state: GrantorState, percent: u8) -> SyncView {
        SyncView {
            device: Some("MacBook".into()),
            lease: Some(LeaseView { state, percent }),
            ..SyncView::default()
        }
    }

    #[test]
    fn chips_follow_the_row() {
        let none = SyncView::default();
        assert_eq!(chip_for(&row("done", None), &none), MeetingChip::Recorded);
        assert_eq!(
            chip_for(&row("ready", None), &none),
            MeetingChip::ProcessedOnPhone
        );
        assert_eq!(chip_for(&row("failed", None), &none), MeetingChip::Failed);
        assert_eq!(
            chip_for(&row("processing", job(0.426, false)), &none),
            MeetingChip::ProcessingOnPhone { percent: 43 }
        );
        assert_eq!(
            chip_for(&row("done", job(0.0, true)), &none),
            MeetingChip::WaitingForModels
        );
    }

    #[test]
    fn a_lease_open_on_the_computer_shows_its_progress() {
        let r = row("processing", None);
        assert_eq!(
            chip_for(&r, &lease(GrantorState::Granted, 42)),
            MeetingChip::FinalOnDesktop { percent: 42 }
        );
        assert_eq!(
            chip_for(&r, &lease(GrantorState::Revoking, 7)),
            MeetingChip::FinalOnDesktop { percent: 7 },
            "until the computer answers the revoke"
        );
    }

    #[test]
    fn a_meeting_the_computer_has_not_taken_waits_for_wifi() {
        let r = row("processing", None);
        assert_eq!(
            chip_for(&r, &lease(GrantorState::Offered, 0)),
            MeetingChip::WaitingForWifi
        );
        assert_eq!(
            chip_for(&r, &lease(GrantorState::Expired, 0)),
            MeetingChip::WaitingForComputer,
            "a computer that stayed away"
        );
        let pending = SyncView {
            device: Some("MacBook".into()),
            pending: true,
            ..SyncView::default()
        };
        assert_eq!(chip_for(&r, &pending), MeetingChip::WaitingForWifi);
    }

    #[test]
    fn a_processed_meeting_the_computer_has_is_synced() {
        let synced = SyncView {
            device: Some("MacBook".into()),
            synced: true,
            ..SyncView::default()
        };
        assert_eq!(chip_for(&row("ready", None), &synced), MeetingChip::Synced);
        // Not processed yet: still just recorded.
        assert_eq!(chip_for(&row("done", None), &synced), MeetingChip::Recorded);
        // The computer's result came back: the lease is done, the meeting ready.
        let mut done = lease(GrantorState::Done, 100);
        done.synced = true;
        assert_eq!(chip_for(&row("ready", None), &done), MeetingChip::Synced);
        // A phone that took the job back shows its own progress.
        assert_eq!(
            chip_for(
                &row("processing", job(0.5, false)),
                &lease(GrantorState::SelfTaken, 0)
            ),
            MeetingChip::ProcessingOnPhone { percent: 50 }
        );
    }
}
