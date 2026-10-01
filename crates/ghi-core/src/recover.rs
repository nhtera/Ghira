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
//!   again; decoding is not resumable).

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
}

fn err(e: ghi_store::StoreError) -> String {
    e.to_string()
}

pub fn recover(store: &Store) -> Result<Recovered, String> {
    let mut out = Recovered::default();
    for d in store.pending_discards().map_err(err)? {
        for k in &d.keep {
            store
                .complete_discard_audio(&d.meeting_gid, k)
                .map_err(err)?;
        }
        store.discard_audio_done(d.id).map_err(err)?;
        out.discards_completed += 1;
    }
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
        for m in page.into_iter().filter(|m| m.status == "recording") {
            let duration = store
                .segments(&m.gid)
                .map_err(err)?
                .iter()
                .map(|s| s.t1_ms)
                .max()
                .unwrap_or(0);
            store.finish_meeting(&m.gid, duration).map_err(err)?;
            store
                .set_meeting_status(&m.gid, "processing")
                .map_err(err)?;
            for kind in [NOTES_LIVE_JOB, FINAL_PASS_JOB] {
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
}
