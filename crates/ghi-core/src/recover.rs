// SPDX-License-Identifier: Apache-2.0
//! Startup recovery, run once after the store opens (the store itself has
//! already cut crashed audio bundles back to their last good page and
//! requeued jobs that were running):
//!
//! - a discard whose audio side never ran is completed [RT-1];
//! - a meeting that was still recording is closed (duration from its last
//!   line) and gets its notes and final pass, like a normal stop: the final
//!   pass re-transcribes everything that reached the bundles;
//! - an import that was still decoding is deleted (the file can be imported
//!   again; decoding is not resumable);
//! - a meeting marked `processing` with no job at all (a crash between the
//!   import marking it and queueing its final pass) gets its final pass;
//! - a meeting a peer recorded (`audio_origin`), or one with an open lease, is
//!   left alone: its pass runs under that lease (doc 07 §8), so queueing
//!   one here would process it twice.

use ghi_store::store::Store;

use crate::notes_job::NOTES_FINAL_JOB;
use crate::session::{FINAL_PASS_JOB, JOB_PAYLOAD_VERSION, NOTES_LIVE_JOB};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recovered {
    pub discards_completed: usize,
    /// Meetings closed after a crash.
    pub meetings: Vec<String>,
    /// Half-imported meetings deleted.
    pub imports_dropped: usize,
    /// Meetings `processing` without a job that got their final pass queued.
    pub jobs_requeued: usize,
}

fn err(e: ghi_store::StoreError) -> String {
    e.to_string()
}

/// The store setting listing the meetings a phone recorded for its computer
/// and has not handed over yet (a JSON array of gids; the sync loop keeps it).
pub const DESKTOP_PENDING_KEY: &str = "sync.desktop_pending";

/// Whether the meeting's processing belongs to a lease: a peer recorded it
/// (this device has no audio of its own to process), a lease is open, or it
/// waits to be handed to the computer.
fn leased_elsewhere(store: &Store, gid: &str) -> Result<bool, String> {
    let waiting = store
        .get_setting(DESKTOP_PENDING_KEY)
        .map_err(err)?
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .is_some_and(|l| l.iter().any(|g| g == gid));
    Ok(waiting
        || store.meeting_audio_origin(gid).map_err(err)?.is_some()
        || store.lease_any_open_for(gid).map_err(err)?)
}

/// The desktop: notes and final pass for what a crash left.
pub fn recover(store: &Store) -> Result<Recovered, String> {
    recover_with_kinds(store, &[NOTES_LIVE_JOB, FINAL_PASS_JOB])
}

/// Applies the audio side of discards whose text side was stored but whose
/// bundle rotation never ran [RT-1]; `meeting` limits it to one meeting.
/// Returns how many were completed. A recording that stops must call this
/// before it queues a final pass, so the pass never reads discarded audio.
pub fn complete_pending_discards(store: &Store, meeting: Option<&str>) -> Result<usize, String> {
    let mut n = 0;
    for d in store.pending_discards().map_err(err)? {
        if meeting.is_some_and(|m| m != d.meeting_gid) {
            continue;
        }
        for k in &d.keep {
            store
                .complete_discard_audio(&d.meeting_gid, k)
                .map_err(err)?;
        }
        store.discard_audio_done(d.id).map_err(err)?;
        n += 1;
    }
    Ok(n)
}

/// Like [`recover`], queueing only `kinds` for closed meetings and for
/// `processing` ones with no job (the phone: `[FINAL_PASS_JOB]`). A kind set
/// without a final pass (the device is below the processing tier) queues
/// nothing: those meetings stay `done`, as recorded.
pub fn recover_with_kinds(store: &Store, kinds: &[&'static str]) -> Result<Recovered, String> {
    let mut out = Recovered {
        discards_completed: complete_pending_discards(store, None)?,
        ..Recovered::default()
    };
    let mut offset = 0;
    loop {
        let page = store.list_meetings(200, offset).map_err(err)?;
        if page.is_empty() {
            break;
        }
        offset += page.len();
        for m in page.iter().filter(|m| m.status == crate::import::IMPORTING) {
            store.delete_meeting(&m.gid).map_err(err)?;
            out.imports_dropped += 1;
            offset -= 1;
        }
        // A sensitive meeting keeps no audio: finish a removal a crash (or a
        // failed stop) interrupted.
        for m in page.iter().filter(|m| m.sensitive) {
            // A failure must not stop the launch (the next one tries again).
            if !store.tracks(&m.gid).map_err(err)?.is_empty()
                && let Err(e) = store.delete_audio(&m.gid)
            {
                log::warn!("removing the audio of a sensitive meeting: {e}");
            }
        }
        for m in page.iter().filter(|m| m.status == "processing") {
            if leased_elsewhere(store, &m.gid)? {
                continue;
            }
            let busy = [NOTES_LIVE_JOB, FINAL_PASS_JOB, NOTES_FINAL_JOB]
                .iter()
                .try_fold(false, |busy, kind| {
                    store.active_job(&m.gid, kind).map(|j| busy || j.is_some())
                })
                .map_err(err)?;
            if !busy && !kinds.contains(&FINAL_PASS_JOB) {
                store.set_meeting_status(&m.gid, "done").map_err(err)?;
            } else if !busy && !store.tracks(&m.gid).map_err(err)?.is_empty() {
                store
                    .enqueue_job(
                        Some(&m.gid),
                        FINAL_PASS_JOB,
                        JOB_PAYLOAD_VERSION,
                        &serde_json::json!({}),
                    )
                    .map_err(err)?;
                out.jobs_requeued += 1;
            } else if !busy {
                // Nothing to wait for and no audio to process: it is as ready as it gets.
                store.set_meeting_status(&m.gid, "ready").map_err(err)?;
            }
        }
        for m in page.into_iter().filter(|m| m.status == "recording") {
            // The final pass needs the audio a sensitive meeting does not keep.
            let kinds = crate::session::job_kinds_at_stop(kinds, m.sensitive);
            let duration = store
                .segments(&m.gid)
                .map_err(err)?
                .iter()
                .map(|s| s.t1_ms)
                .max()
                .unwrap_or(0);
            store.finish_meeting(&m.gid, duration).map_err(err)?;
            store
                .set_meeting_status(
                    &m.gid,
                    if m.sensitive {
                        // Its transcript is final as recorded.
                        "ready"
                    } else if kinds.is_empty() {
                        "done"
                    } else {
                        "processing"
                    },
                )
                .map_err(err)?;
            for &kind in &kinds {
                let busy = store.active_job(&m.gid, kind).map_err(err)?.is_some()
                    || (kind == NOTES_LIVE_JOB
                        && store
                            .active_job(&m.gid, NOTES_FINAL_JOB)
                            .map_err(err)?
                            .is_some());
                if !busy {
                    store
                        .enqueue_job(
                            Some(&m.gid),
                            kind,
                            JOB_PAYLOAD_VERSION,
                            &serde_json::json!({}),
                        )
                        .map_err(err)?;
                }
            }
            out.meetings.push(m.gid);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_store::keys::{MemoryKeyStore, Protection};
    use ghi_store::store::{NewMeeting, NewSegment, TrackKind};
    use std::sync::Arc;

    #[test]
    fn a_crashed_recording_is_closed_and_processed_and_a_discard_completed() {
        let tmp = tempfile::tempdir().unwrap();
        let keys = Arc::new(MemoryKeyStore::default());
        let gid;
        {
            let store = Store::open(tmp.path(), keys.clone(), Protection::default()).unwrap();
            gid = store.create_meeting(NewMeeting::default()).unwrap().gid;
            let mut w = store.open_track(&gid, TrackKind::Mic).unwrap();
            for i in 0..6u8 {
                w.append(&[i]).unwrap();
            }
            w.sync(true).unwrap();
            store
                .add_segment(
                    &gid,
                    NewSegment {
                        t0_ms: 0,
                        t1_ms: 4_200,
                        text: "xin chào".into(),
                        ..Default::default()
                    },
                )
                .unwrap();
            let keep = ghi_store::edits::KeepPages {
                kind: TrackKind::Mic,
                pages: 3,
                prefix: Some(w.prefix_hex()),
            };
            store
                .discard_after(&gid, 3_000, 6_000, &[keep], true)
                .unwrap();
            // Crash: the writer is dropped unfinished, the discard's audio
            // side never ran.
            drop(w);
        }
        let store = Store::open(tmp.path(), keys, Protection::default()).unwrap();
        let r = recover(&store).unwrap();
        assert_eq!(r.discards_completed, 1);
        assert_eq!(r.meetings, std::slice::from_ref(&gid));
        let m = store.get_meeting(&gid).unwrap();
        assert_eq!(m.status, "processing");
        assert_eq!(
            store
                .open_bundle(&gid, TrackKind::Mic)
                .unwrap()
                .page_count(),
            3
        );
        let kinds: Vec<String> = store
            .jobs_for_meeting(&gid)
            .unwrap()
            .into_iter()
            .map(|j| j.kind)
            .collect();
        assert_eq!(kinds, [NOTES_LIVE_JOB, FINAL_PASS_JOB]);
        // Idempotent.
        assert_eq!(recover(&store).unwrap(), Recovered::default());
    }

    #[test]
    fn a_crashed_sensitive_recording_loses_its_audio_and_gets_no_final_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let keys = Arc::new(MemoryKeyStore::default());
        let gid;
        {
            let store = Store::open(tmp.path(), keys.clone(), Protection::default()).unwrap();
            gid = store.create_meeting(NewMeeting::default()).unwrap().gid;
            let mut w = store.open_track(&gid, TrackKind::Mic).unwrap();
            w.append(&[1]).unwrap();
            w.sync(true).unwrap();
            // Turned on mid-recording, then the app died.
            store.set_sensitive(&gid, true).unwrap();
            drop(w);
        }
        let store = Store::open(tmp.path(), keys, Protection::default()).unwrap();
        recover(&store).unwrap();
        assert!(!store.audio_available(&gid).unwrap());
        let kinds: Vec<String> = store
            .jobs_for_meeting(&gid)
            .unwrap()
            .into_iter()
            .map(|j| j.kind)
            .collect();
        assert_eq!(kinds, [NOTES_FINAL_JOB]);
        assert_eq!(store.get_meeting(&gid).unwrap().status, "ready");
        // The phone (final pass only): nothing to queue, the meeting is done.
        let other = store.create_meeting(NewMeeting::default()).unwrap().gid;
        store.set_sensitive(&other, true).unwrap();
        recover_with_kinds(&store, &[FINAL_PASS_JOB]).unwrap();
        assert!(store.jobs_for_meeting(&other).unwrap().is_empty());
        assert_eq!(store.get_meeting(&other).unwrap().status, "ready");
    }

    #[test]
    fn a_processing_meeting_with_no_audio_and_no_job_is_ready() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        let gid = store.create_meeting(NewMeeting::default()).unwrap().gid;
        store.set_meeting_status(&gid, "processing").unwrap();
        recover(&store).unwrap();
        assert_eq!(store.get_meeting(&gid).unwrap().status, "ready");
        assert!(store.jobs_for_meeting(&gid).unwrap().is_empty());
    }

    /// A `processing` meeting with audio and no job.
    fn processing_with_audio(store: &Store) -> String {
        let gid = store.create_meeting(NewMeeting::default()).unwrap().gid;
        let mut w = store.open_track(&gid, TrackKind::Mic).unwrap();
        w.append(&[1]).unwrap();
        w.sync(true).unwrap();
        drop(w);
        store.set_meeting_status(&gid, "processing").unwrap();
        gid
    }

    #[test]
    fn peer_recorded_and_leased_meetings_get_no_final_pass_but_local_ones_do() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        let (local, hub, phone) = (
            processing_with_audio(&store),
            processing_with_audio(&store),
            processing_with_audio(&store),
        );
        // The hub: a phone recorded it.
        store
            .set_meeting_audio_origin_for_tests(&hub, Some(3))
            .unwrap();
        // The phone: it granted the pass to the desktop.
        store
            .insert_lease_for_tests("lease-1", &phone, 1, "granted")
            .unwrap();

        let out = recover_with_kinds(&store, &[FINAL_PASS_JOB]).unwrap();
        assert_eq!(out.jobs_requeued, 1);
        assert!(store.jobs_for_meeting(&hub).unwrap().is_empty());
        assert!(store.jobs_for_meeting(&phone).unwrap().is_empty());
        assert_eq!(store.jobs_for_meeting(&local).unwrap().len(), 1);
        for g in [&hub, &phone] {
            assert_eq!(store.get_meeting(g).unwrap().status, "processing");
        }

        // A closed lease no longer holds the meeting back.
        let closed = processing_with_audio(&store);
        store
            .insert_lease_for_tests("lease-2", &closed, 1, "done")
            .unwrap();
        recover_with_kinds(&store, &[FINAL_PASS_JOB]).unwrap();
        assert_eq!(store.jobs_for_meeting(&closed).unwrap().len(), 1);
    }

    #[test]
    fn a_meeting_waiting_to_be_handed_to_the_computer_is_not_processed_here() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        let (waiting, local) = (processing_with_audio(&store), processing_with_audio(&store));
        store
            .set_setting(DESKTOP_PENDING_KEY, &serde_json::json!([waiting]))
            .unwrap();
        let out = recover_with_kinds(&store, &[FINAL_PASS_JOB]).unwrap();
        assert_eq!(out.jobs_requeued, 1);
        assert!(store.jobs_for_meeting(&waiting).unwrap().is_empty());
        assert_eq!(store.jobs_for_meeting(&local).unwrap().len(), 1);
    }

    /// A removal that fails must not stop the launch.
    #[cfg(unix)]
    #[test]
    fn a_failed_audio_removal_of_a_sensitive_meeting_does_not_stop_recovery() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        let gid = store.create_meeting(NewMeeting::default()).unwrap().gid;
        store.set_meeting_status(&gid, "done").unwrap();
        let mut w = store.open_track(&gid, TrackKind::Mic).unwrap();
        w.append(&[1]).unwrap();
        store.finish_track(&gid, TrackKind::Mic, w).unwrap();
        store.set_sensitive(&gid, true).unwrap();
        // The bundle folder cannot be emptied.
        let dir = tmp.path().join("bundles").join(&gid);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let blocked = std::fs::remove_dir_all(&dir).is_err();
        let r = recover(&store);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        if blocked {
            assert!(r.is_ok(), "{r:?}");
            assert!(
                store.audio_available(&gid).unwrap(),
                "left for the next launch"
            );
        }
        // The next launch finishes the removal.
        recover(&store).unwrap();
        assert!(!store.audio_available(&gid).unwrap());
    }

    fn open_with_crashed_meeting() -> (tempfile::TempDir, Store, String) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        let gid = store.create_meeting(NewMeeting::default()).unwrap().gid;
        let mut w = store.open_track(&gid, TrackKind::Mic).unwrap();
        w.append(&[1]).unwrap();
        w.sync(true).unwrap();
        drop(w);
        (tmp, store, gid)
    }

    #[test]
    fn the_phone_queues_only_the_final_pass() {
        let (_tmp, store, gid) = open_with_crashed_meeting();
        let r = recover_with_kinds(&store, &[FINAL_PASS_JOB]).unwrap();
        assert_eq!(r.meetings, std::slice::from_ref(&gid));
        assert_eq!(store.get_meeting(&gid).unwrap().status, "processing");
        let kinds: Vec<String> = store
            .jobs_for_meeting(&gid)
            .unwrap()
            .into_iter()
            .map(|j| j.kind)
            .collect();
        assert_eq!(kinds, [FINAL_PASS_JOB]);
    }

    #[test]
    fn below_tier_nothing_is_queued_and_meetings_stay_as_recorded() {
        let (_tmp, store, gid) = open_with_crashed_meeting();
        recover_with_kinds(&store, &[]).unwrap();
        assert_eq!(store.get_meeting(&gid).unwrap().status, "done");
        assert!(store.jobs_for_meeting(&gid).unwrap().is_empty());
        // A meeting `processing` with no job is settled the same way.
        let other = store.create_meeting(NewMeeting::default()).unwrap().gid;
        let mut w = store.open_track(&other, TrackKind::Mic).unwrap();
        w.append(&[1]).unwrap();
        w.sync(true).unwrap();
        drop(w);
        store.finish_meeting(&other, 1_000).unwrap();
        store.set_meeting_status(&other, "processing").unwrap();
        recover_with_kinds(&store, &[]).unwrap();
        assert_eq!(store.get_meeting(&other).unwrap().status, "done");
        assert!(store.jobs_for_meeting(&other).unwrap().is_empty());
    }
}
