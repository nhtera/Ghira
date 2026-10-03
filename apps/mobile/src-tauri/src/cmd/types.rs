// SPDX-License-Identifier: Apache-2.0
//! Types shared by the mobile commands and events (phase 16 contracts).

use serde::{Deserialize, Serialize};
use specta::Type;

/// Where a recording is processed after it stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ProcessingTarget {
    /// The final pass runs on this phone (needs the speech models and a capable device).
    Phone,
    /// Phase 15: a paired desktop. Listed but disabled until pairing exists.
    Desktop,
    /// Notes through the user's own cloud key, transcript text only, per
    /// meeting and after a preview. The transcript itself is still made on the phone.
    Cloud,
}

/// The status chip on a meeting row (M3). v1 produces only the first five;
/// the last three are phase 15 and are never sent before pairing exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MeetingChip {
    Recorded,
    ProcessingOnPhone { percent: u8 },
    ProcessedOnPhone,
    WaitingForModels,
    Failed,
    // Phase 15.
    Synced,
    WaitingForWifi,
    FinalOnDesktop { percent: u8 },
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
/// v1 never produces the phase-15 chips.
pub fn chip_for(row: &ghi_app::library::MeetingRow) -> MeetingChip {
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

    #[test]
    fn chips_follow_the_row() {
        assert_eq!(chip_for(&row("done", None)), MeetingChip::Recorded);
        assert_eq!(chip_for(&row("ready", None)), MeetingChip::ProcessedOnPhone);
        assert_eq!(chip_for(&row("failed", None)), MeetingChip::Failed);
        assert_eq!(
            chip_for(&row("processing", job(0.426, false))),
            MeetingChip::ProcessingOnPhone { percent: 43 }
        );
        assert_eq!(
            chip_for(&row("done", job(0.0, true))),
            MeetingChip::WaitingForModels
        );
    }
}
