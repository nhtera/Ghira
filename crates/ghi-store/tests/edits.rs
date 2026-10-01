// SPDX-License-Identifier: Apache-2.0
//! Phase 8 edits: speaker rename/merge/split/not-a-person, discard [RT-1],
//! import hashes, and job checkpoint/release.

mod common;

use ghi_store::bundle::BundleReader;
use ghi_store::edits::KeepPages;
use ghi_store::jobs::JobState;
use ghi_store::search::SearchQuery;
use ghi_store::store::{
    MarkTag, NewNoteBlock, NewSegment, NewSpeaker, Provenance, Store, TrackKind,
};
use serde_json::json;

fn line(store: &Store, m: &str, speaker: &str, t0: i64, t1: i64, text: &str) -> String {
    store
        .add_segment(
            m,
            NewSegment {
                speaker_gid: Some(speaker.into()),
                ..common::seg(t0, t1, text)
            },
        )
        .unwrap()
        .gid
}

fn keep(kind: TrackKind, pages: u32, prefix: Option<String>) -> KeepPages {
    KeepPages {
        kind,
        pages,
        prefix,
    }
}

fn speaker(store: &Store, m: &str, idx: i64) -> String {
    store
        .add_speaker(
            m,
            NewSpeaker {
                label_idx: idx,
                color_slot: idx,
                ..Default::default()
            },
        )
        .unwrap()
}

#[test]
fn rename_merge_split_and_not_a_person() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "1:1");
    let me = store
        .add_speaker(
            &m,
            NewSpeaker {
                label_idx: 0,
                is_me: true,
                ..Default::default()
            },
        )
        .unwrap();
    let a = speaker(&store, &m, 1);
    let b = speaker(&store, &m, 2);
    let l1 = line(&store, &m, &a, 0, 1_000, "chào anh");
    let l2 = line(&store, &m, &b, 1_000, 2_000, "chào em");
    let l3 = line(&store, &m, &a, 2_000, 3_000, "bắt đầu nhé");

    store.rename_speaker(&a, Some("  Lan ")).unwrap();
    let find = |gid: &str| {
        store
            .speakers(&m)
            .unwrap()
            .into_iter()
            .find(|s| s.gid == gid)
            .unwrap()
    };
    assert_eq!(find(&a).display_name.as_deref(), Some("Lan"));
    store.rename_speaker(&a, Some("   ")).unwrap();
    assert_eq!(find(&a).display_name, None, "blank clears");

    // b was Lan too.
    store.merge_speakers(&b, &a).unwrap();
    assert_eq!(find(&b).merged_into.as_deref(), Some(a.as_str()));
    let owner = |seg: &str| {
        store
            .segments(&m)
            .unwrap()
            .into_iter()
            .find(|s| s.gid == seg)
            .unwrap()
            .speaker_gid
    };
    assert_eq!(owner(&l2).as_deref(), Some(a.as_str()));
    assert!(store.merge_speakers(&a, &a).is_err());

    // Me merged into a keeps Me.
    store.merge_speakers(&me, &a).unwrap();
    assert!(find(&a).is_me && !find(&me).is_me);

    // l3 was someone else after all.
    let c = store
        .split_speaker(&a, std::slice::from_ref(&l3), 4)
        .unwrap();
    assert_eq!(owner(&l3).as_deref(), Some(c.as_str()));
    assert_eq!(owner(&l1).as_deref(), Some(a.as_str()));
    assert!(
        store
            .split_speaker(&a, std::slice::from_ref(&l3), 4)
            .is_err(),
        "l3 is no longer a's"
    );
    assert_eq!(find(&c).label_idx, 3);

    store.set_speaker_not_person(&c, true).unwrap();
    assert!(find(&c).not_person);
    store.set_segment_speaker(&l3, None).unwrap();
    assert_eq!(owner(&l3), None);
}

#[test]
fn discard_leaves_no_trace_in_transcript_search_marks_or_notes() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "budget");
    let a = speaker(&store, &m, 1);
    let late = speaker(&store, &m, 2);
    line(&store, &m, &a, 0, 4_000, "ngân sách quý ba giữ nguyên");
    // Straddles the cut at 5 s: goes entirely.
    line(&store, &m, &a, 4_500, 5_500, "lương thưởng bí mật");
    line(&store, &m, &late, 6_000, 8_000, "mật khẩu wifi là hoa sen");
    store.add_mark(&m, 1_000, MarkTag::Star).unwrap();
    store.add_mark(&m, 7_000, MarkTag::Decision).unwrap();
    let early = store.anchor_for_range(&m, 0, 4_000).unwrap();
    let window = store.anchor_for_range(&m, 6_000, 8_000).unwrap();
    let note = |body: &str, anchors| NewNoteBlock {
        kind: "notepad".into(),
        provenance: Provenance::User,
        body: body.into(),
        anchors,
        pinned: false,
    };
    store
        .add_note_block(&m, note("giữ ngân sách", vec![early]))
        .unwrap();
    store
        .add_note_block(&m, note("hoa sen wifi", vec![window]))
        .unwrap();
    let mut audio = store.open_track(&m, TrackKind::Mic).unwrap();
    for i in 0..8u8 {
        audio.append(&[i; 16]).unwrap();
    }

    let rep = store
        .discard_after(&m, 5_000, 8_000, &[keep(TrackKind::Mic, 5, None)], true)
        .unwrap();
    assert_eq!(
        (rep.segments, rep.marks, rep.notes, rep.speakers),
        (2, 1, 1, 1)
    );
    // Audio side: the live writer rotates down to the kept pages.
    audio.rotate(5).unwrap();
    store.finish_track(&m, TrackKind::Mic, audio).unwrap();
    store.discard_audio_done(rep.id).unwrap();

    let texts: Vec<String> = store
        .segments(&m)
        .unwrap()
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(texts, ["ngân sách quý ba giữ nguyên"]);
    for q in ["luong thuong", "hoa sen", "mat khau"] {
        assert!(
            store.search(&SearchQuery::new(q)).unwrap().is_empty(),
            "{q} still found"
        );
    }
    assert_eq!(
        store.search(&SearchQuery::new("ngan sach")).unwrap().len(),
        2
    );
    assert_eq!(store.marks(&m).unwrap().len(), 1);
    assert_eq!(store.note_blocks(&m).unwrap().len(), 1);
    assert_eq!(
        store.speakers(&m).unwrap().len(),
        1,
        "the late speaker had no line left"
    );
    assert_eq!(store.discarded_spans(&m).unwrap(), [(5_000, 8_000)]);
    assert!(store.pending_discards().unwrap().is_empty());
    let kinds: Vec<String> = store
        .tombstones_since(0)
        .unwrap()
        .into_iter()
        .map(|t| t.kind)
        .collect();
    for k in ["segment", "mark", "note", "speaker"] {
        assert!(kinds.iter().any(|x| x == k), "no {k} tombstone");
    }
    let path = store.bundle_path(&m, TrackKind::Mic).unwrap();
    assert!(path.exists());
    let audio = store.open_bundle(&m, TrackKind::Mic).unwrap();
    assert_eq!(audio.page_count(), 5);
    let _ = BundleReader::page; // the reader API used above
}

#[test]
fn a_pending_discard_is_completed_after_a_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "crash");
    let mut audio = store.open_track(&m, TrackKind::Mic).unwrap();
    for i in 0..6u8 {
        audio.append(&[i]).unwrap();
    }
    store.finish_track(&m, TrackKind::Mic, audio).unwrap();
    let prefix =
        ghi_store::bundle::file_prefix_hex(&store.bundle_path(&m, TrackKind::Mic).unwrap())
            .unwrap();
    store
        .discard_after(
            &m,
            3_000,
            6_000,
            &[keep(TrackKind::Mic, 3, Some(prefix.clone()))],
            true,
        )
        .unwrap();
    // Crash before the audio side: at startup the core finds it pending.
    let pending = store.pending_discards().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].keep, [keep(TrackKind::Mic, 3, Some(prefix))]);
    for k in &pending[0].keep {
        assert_eq!(
            store
                .complete_discard_audio(&pending[0].meeting_gid, k)
                .unwrap(),
            Some(3)
        );
        // Already rotated (a new prefix): never cut again.
        assert_eq!(
            store
                .complete_discard_audio(&pending[0].meeting_gid, k)
                .unwrap(),
            None
        );
    }
    store.discard_audio_done(pending[0].id).unwrap();
    assert_eq!(
        store.open_bundle(&m, TrackKind::Mic).unwrap().page_count(),
        3
    );
    assert!(store.pending_discards().unwrap().is_empty());
}

#[test]
fn import_hash_finds_duplicates() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "zoom");
    let h = "AB".repeat(32);
    assert_eq!(store.meeting_by_source_hash(&h).unwrap(), None);
    store.set_source_hash(&m, &h).unwrap();
    assert_eq!(
        store.meeting_by_source_hash(&h.to_lowercase()).unwrap(),
        Some(m.clone())
    );
    assert!(store.set_source_hash(&m, "not a hash").is_err());
}

#[test]
fn preempted_jobs_keep_their_attempts_and_resume_point() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "jobs");
    let id = store
        .enqueue_job(Some(&m), "final_pass", 1, &json!({"stage": 0}))
        .unwrap();
    for round in 0..8 {
        let job = store
            .claim_next_job("final_pass", 1)
            .unwrap()
            .expect("claimable");
        assert_eq!(job.payload["stage"], json!(round.min(1)));
        store
            .checkpoint_job(id, 0.4, &json!({"stage": 1, "chunk": round}))
            .unwrap();
        // A recording starts: hand it back.
        store
            .release_job(id, &json!({"stage": 1, "chunk": round}))
            .unwrap();
    }
    let job = store.job(id).unwrap();
    assert_eq!((job.state, job.attempts), (JobState::Queued, 0));
    assert_eq!(job.payload["chunk"], json!(7));
    assert!(
        store
            .checkpoint_job(id, 0.5, &json!({"x": "free text with spaces"}))
            .is_err()
    );
    assert!(store.release_job(id, &json!({})).is_err(), "not running");
    assert_eq!(
        store.active_job(&m, "final_pass").unwrap().map(|j| j.id),
        Some(id)
    );
    assert_eq!(store.active_job(&m, "import").unwrap(), None);
}
