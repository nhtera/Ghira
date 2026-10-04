// SPDX-License-Identifier: Apache-2.0
//! Sensitive meeting mode (doc 02, P1): the meeting keeps its transcript and
//! nothing else sensitive. No audio is saved, cloud AI is refused for it
//! (`ghi-core::cloud`, `ghi-net`'s gate), and no voice is learned from it
//! (`voice_step`, `voice_job`).
//!
//! A recording turns the mode on through [`crate::session::Session::set_sensitive`]
//! (one way: part of its audio is gone). A stored meeting goes through
//! [`set_stored`]. The mode can be turned off on a stored meeting (the
//! transcript may then use the cloud again); the audio stays deleted.

use ghi_store::jobs::JobState;
use ghi_store::store::Store;

use crate::voice_job::VOICE_LEARN_JOB;

fn err(e: ghi_store::StoreError) -> String {
    e.to_string()
}

/// What turning the mode on removed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Audio tracks deleted.
    pub tracks: u32,
}

/// Refusals of [`set_stored`] (stable words, the UI words them).
pub const ERR_NO_TRANSCRIPT: &str = "noTranscript";
pub const ERR_PENDING: &str = "transcriptPending";

/// Sets sensitive mode on a meeting that is not recording.
///
/// On: refused (`noTranscript`) when the meeting has no transcript, since
/// nothing would be kept, and (`transcriptPending`) while its final pass is
/// queued or running, since that pass reads the audio and its result is the
/// transcript. Otherwise the flag, then the audio goes (files, rows,
/// waveform), queued `voice_learn` jobs are cancelled and the voices already
/// learned from this meeting are dropped (Me's exemplars, the speakers' stored
/// voices). The flag and the pass check are one conditional update, so a pass
/// that starts meanwhile sees the flag (`final_pass` checks it first) and one
/// that already runs refuses the change. Off: the flag only.
// TODO(third-party voices): also drop the exemplars added to other people's
// profiles from this meeting once those profiles are allowed
// (`THIRD_PARTY_APPROVED`, voice_job.rs); they are hard-off today.
pub fn set_stored(store: &Store, meeting: &str, on: bool) -> Result<Applied, String> {
    let m = store.get_meeting(meeting).map_err(err)?;
    if m.status == "recording" {
        return Err("the meeting is recording: use the live toggle".into());
    }
    if !on {
        store.set_sensitive(meeting, false).map_err(err)?;
        return Ok(Applied::default());
    }
    if store.segments(meeting).map_err(err)?.is_empty() {
        return Err(ERR_NO_TRANSCRIPT.into());
    }
    if !store
        .set_sensitive_unless_final_pass_active(meeting)
        .map_err(err)?
    {
        return Err(ERR_PENDING.into());
    }
    let tracks = store.delete_audio(meeting).map_err(err)?;
    for j in store.active_jobs().map_err(err)? {
        if j.meeting_gid.as_deref() == Some(meeting)
            && j.state == JobState::Queued
            && j.kind == VOICE_LEARN_JOB
        {
            store.cancel_job(j.id).map_err(err)?;
        }
    }
    store.drop_me_exemplars_from(meeting).map_err(err)?;
    for (speaker, _) in store.speaker_voices(meeting).map_err(err)? {
        store.clear_speaker_voice(&speaker).map_err(err)?;
    }
    Ok(Applied { tracks })
}
