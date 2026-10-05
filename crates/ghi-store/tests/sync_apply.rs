// SPDX-License-Identifier: Apache-2.0
//! Slice 15-C2: the merge engine (doc 07 §7.3-§7.5). Two or three stores play
//! hub and spokes; records are encoded by the sender's store (or forged from
//! them, sealed under the meeting key) and applied to the receiver.

mod common;

use std::sync::atomic::{AtomicU8, Ordering};

use ghi_store::StoreError;
use ghi_store::rowcrypt::{Dek, row_aad, seal_text};
use ghi_store::search::SearchQuery;
use ghi_store::store::{
    MarkTag, NewActionItem, NewNoteBlock, NewSegment, NewSpeaker, Provenance, Store,
};
use ghi_store::sync::apply::{ApplyOutcome, ApplyResult};
use ghi_store::sync::devices::{DeviceRole, NewDevice};
use ghi_store::sync::records::{
    Bytes, ConflictCopyRec, MeetingTagRec, Record, SettingRec, SyncTombstone, TombCause, TrackRec,
    Version,
};
use ghi_store::sync::rules;

use ApplyOutcome::{Accepted, Merged, Parked, Tombstoned};

static NEXT: AtomicU8 = AtomicU8::new(1);

/// One device: a store and its own sync gid.
struct Node {
    _tmp: tempfile::TempDir,
    keys: common::Keys,
    store: Store,
    gid: String,
}

impl Node {
    /// A second connection on the same database, for what the API hides.
    fn raw(&self) -> rusqlite::Connection {
        ghi_store::db::open(
            &self._tmp.path().join("ghira.db"),
            &common::db_key(&self.keys),
        )
        .unwrap()
    }
}

fn node() -> Node {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let gid = store.sync_device_gid().unwrap();
    Node {
        _tmp: tmp,
        keys,
        store,
        gid,
    }
}

fn new_device(of: &Node, role: DeviceRole) -> NewDevice {
    NewDevice {
        gid: of.gid.clone(),
        name: "Peer".into(),
        platform: "mac".into(),
        role,
        static_pub: [NEXT.fetch_add(1, Ordering::SeqCst); 32],
    }
}

/// Pairs `spoke` with `hub` on both sides.
fn link(hub: &Node, spoke: &Node) {
    hub.store
        .pin_device(&new_device(spoke, DeviceRole::Spoke), &[1; 32])
        .unwrap();
    spoke
        .store
        .pin_device(&new_device(hub, DeviceRole::Hub), &[1; 32])
        .unwrap();
}

fn rec(n: &Node, kind: &str, gid: &str) -> Record {
    n.store
        .encode_record(kind, gid, kind == "meeting")
        .unwrap()
        .unwrap_or_else(|| panic!("no {kind} {gid}"))
}

fn push(to: &Node, from: &Node, recs: Vec<Record>) -> ApplyResult {
    to.store.apply_rows(&from.gid, &recs).expect("apply")
}

fn outcomes(r: &ApplyResult) -> Vec<ApplyOutcome> {
    r.results.iter().map(|(_, o)| *o).collect()
}

fn dek_of(r: &Record) -> Dek {
    let Record::Meeting(m) = r else {
        panic!("not a meeting")
    };
    Dek::from_bytes(m.dek.as_ref().expect("a key").0.clone().try_into().unwrap())
}

fn seal(dek: &Dek, table: &str, col: &str, gid: &str, text: &str) -> Bytes {
    Bytes(seal_text(dek, text, &row_aad(table, col, gid)))
}

fn ver(lamport: i64, origin: &str) -> Version {
    Version {
        lamport,
        origin: origin.into(),
    }
}

fn lamport(n: &Node, kind: &str, gid: &str) -> i64 {
    rec(n, kind, gid).version().lamport
}

fn note(s: &Node, m: &str, body: &str, prov: Provenance, pinned: bool) -> String {
    s.store
        .add_note_block(
            m,
            NewNoteBlock {
                kind: "paragraph".into(),
                provenance: prov,
                body: body.into(),
                anchors: vec![],
                pinned,
            },
        )
        .unwrap()
        .gid
}

fn user_note(s: &Node, m: &str, body: &str) -> String {
    note(s, m, body, Provenance::User, false)
}

fn bodies(n: &Node, m: &str) -> Vec<String> {
    n.store
        .note_blocks(m)
        .unwrap()
        .into_iter()
        .map(|b| b.body)
        .collect()
}

fn state(n: &Node, kind: &str, gids: &[&str]) -> Vec<Option<Record>> {
    gids.iter()
        .map(|g| n.store.encode_record(kind, g, false).unwrap())
        .collect()
}

/// A meeting on `s1` with a speaker, one segment, one note, one action item
/// and one mark. Returns the gids.
struct World {
    m: String,
    speaker: String,
    seg: String,
    note: String,
    action: String,
    mark: String,
}

fn world(s: &Node) -> World {
    let m = common::meeting(&s.store, "Họp kế hoạch");
    let speaker = s
        .store
        .add_speaker(
            &m,
            NewSpeaker {
                label_idx: 0,
                display_name: Some("Linh".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let seg = s
        .store
        .add_segments(
            &m,
            vec![NewSegment {
                t0_ms: 0,
                t1_ms: 900,
                text: "Chốt kế hoạch quý bốn".into(),
                speaker_gid: Some(speaker.clone()),
                ..Default::default()
            }],
        )
        .unwrap()[0]
        .gid
        .clone();
    let note = user_note(s, &m, "Ghi chú đầu tiên");
    let action = s
        .store
        .add_action_item(
            &m,
            NewActionItem {
                text: "Gửi báo cáo".into(),
                due_text: Some("thứ sáu".into()),
                ..Default::default()
            },
        )
        .unwrap()
        .gid;
    let mark = s.store.add_mark(&m, 500, MarkTag::Star).unwrap().gid;
    World {
        m,
        speaker,
        seg,
        note,
        action,
        mark,
    }
}

fn world_records(s: &Node, w: &World) -> Vec<Record> {
    vec![
        rec(s, "meeting", &w.m),
        rec(s, "speaker", &w.speaker),
        rec(s, "segment", &w.seg),
        rec(s, "note", &w.note),
        rec(s, "action_item", &w.action),
        rec(s, "mark", &w.mark),
    ]
}

fn link_gids(n: &Node) -> Vec<String> {
    n.raw()
        .prepare("SELECT gid FROM meeting_tags ORDER BY gid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn tomb(gid: &str, kind: &str, from: &Node, cause: Option<TombCause>) -> SyncTombstone {
    SyncTombstone {
        gid: gid.into(),
        kind: kind.into(),
        lamport: 5,
        origin: from.gid.clone(),
        cause,
    }
}

// ------------------------------------------------------------------ accept

#[test]
fn sync_a_new_meeting_and_its_children_are_accepted_and_indexed() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    let r = push(&hub, &s1, world_records(&s1, &w));
    assert_eq!(outcomes(&r), vec![Accepted; 6]);

    assert_eq!(hub.store.get_meeting(&w.m).unwrap().title, "Họp kế hoạch");
    assert_eq!(
        hub.store.segments(&w.m).unwrap()[0].text,
        "Chốt kế hoạch quý bốn"
    );
    assert_eq!(bodies(&hub, &w.m), vec!["Ghi chú đầu tiên"]);
    assert_eq!(
        hub.store.action_items(&w.m).unwrap()[0].due_text.as_deref(),
        Some("thứ sáu")
    );
    assert_eq!(hub.store.marks(&w.m).unwrap().len(), 1);
    // The FTS rows were rebuilt from the decrypted text, accents folded.
    let hits = hub.store.search(&SearchQuery::new("ke hoach")).unwrap();
    assert!(hits.iter().any(|h| h.item_gid == w.seg));
    assert!(hub.store.index_gen(&w.m).unwrap() > 0);
    // The sender holds the key: this meeting is in its wipe scope.
    assert_eq!(hub.store.peer_meetings(&s1.gid).unwrap(), vec![w.m.clone()]);
}

#[test]
fn sync_encode_then_apply_round_trips_every_kind() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    let tag = s1.store.create_tag("Quý bốn").unwrap().gid;
    s1.store
        .tag_meetings(std::slice::from_ref(&w.m), &tag)
        .unwrap();
    let folder = s1.store.create_folder("Dự án").unwrap().gid;
    s1.store
        .set_meeting_folder(std::slice::from_ref(&w.m), Some(&folder))
        .unwrap();
    let link_gid = link_gids(&s1).pop().expect("a link");
    let mut recs = vec![rec(&s1, "folder", &folder), rec(&s1, "tag", &tag)];
    recs.extend(world_records(&s1, &w));
    recs.push(rec(&s1, "meeting_tag", &link_gid));
    let r = push(&hub, &s1, recs.clone());
    assert!(outcomes(&r).iter().all(|o| *o == Accepted), "{r:?}");
    for rec_in in recs {
        let Record::Meeting(_) = &rec_in else {
            let kind = rec_in.kind().log_kind();
            let back = hub.store.encode_record(kind, rec_in.gid(), false).unwrap();
            assert_eq!(back, Some(rec_in), "{kind}");
            continue;
        };
        let back = hub
            .store
            .encode_record("meeting", rec_in.gid(), true)
            .unwrap();
        assert_eq!(back, Some(rec_in), "meeting");
    }
    assert_eq!(
        hub.store.get_meeting(&w.m).unwrap().folder_gid,
        Some(folder)
    );
}

#[test]
fn sync_duplicate_delivery_is_a_no_op_with_no_second_copy() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    let batch = world_records(&s1, &w);
    push(&hub, &s1, batch.clone());
    let kinds = [
        "meeting",
        "speaker",
        "segment",
        "note",
        "action_item",
        "mark",
    ];
    let gids = [&w.m, &w.speaker, &w.seg, &w.note, &w.action, &w.mark];
    let before: Vec<_> = kinds
        .iter()
        .zip(gids)
        .map(|(k, g)| state(&hub, k, &[g]))
        .collect();
    let lamport_before = hub.store.pending_count().unwrap();
    let again = push(&hub, &s1, batch);
    assert_eq!(outcomes(&again), vec![Accepted; 6]);
    let after: Vec<_> = kinds
        .iter()
        .zip(gids)
        .map(|(k, g)| state(&hub, k, &[g]))
        .collect();
    assert_eq!(before, after);
    assert!(hub.store.conflict_copies(&w.m).unwrap().is_empty());
    assert_eq!(hub.store.pending_count().unwrap(), lamport_before);
}

// -------------------------------------------------------------- tombstones

#[test]
fn sync_tombstone_outranks_a_higher_lamport_edit_and_survives_a_base_reset() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    push(&hub, &s1, world_records(&s1, &w));

    let t = tomb(&w.seg, "segment", &s1, Some(TombCause::User));
    let r = hub.store.apply_tombs(&s1.gid, &[t]).unwrap();
    assert_eq!(r.applied, vec![w.seg.clone()]);
    assert!(hub.store.segments(&w.m).unwrap().is_empty());
    assert!(hub.store.is_tombstoned(&w.seg).unwrap());

    // A later edit (lamport + 100) and a re-pair (base reset to None).
    let Record::Segment(mut late) = rec(&s1, "segment", &w.seg) else {
        unreachable!()
    };
    late.version.lamport += 100;
    late.base = None;
    let r = push(&hub, &s1, vec![Record::Segment(late)]);
    assert_eq!(outcomes(&r), vec![Tombstoned]);
    assert!(hub.store.segments(&w.m).unwrap().is_empty());
    // The same tombstone again changes nothing.
    let r = hub
        .store
        .apply_tombs(&s1.gid, &[tomb(&w.seg, "segment", &s1, None)])
        .unwrap();
    assert!(r.applied.is_empty());
}

#[test]
fn sync_a_meeting_tombstone_kills_its_children_and_shreds_the_meeting() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    push(&hub, &s1, world_records(&s1, &w));

    let r = hub
        .store
        .apply_tombs(
            &s1.gid,
            &[tomb(&w.m, "meeting", &s1, Some(TombCause::User))],
        )
        .unwrap();
    assert_eq!(r.applied, vec![w.m.clone()]);
    assert!(matches!(
        hub.store.get_meeting(&w.m),
        Err(StoreError::NotFound { .. })
    ));
    for g in [&w.speaker, &w.seg, &w.note, &w.action, &w.mark] {
        assert!(hub.store.is_tombstoned(g).unwrap(), "{g}");
    }
    // A child that arrives later (even at a higher lamport) is dropped.
    let Record::Note(mut late) = rec(&s1, "note", &w.note) else {
        unreachable!()
    };
    late.version.lamport += 100;
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![Record::Note(late)])),
        vec![Tombstoned]
    );
    // And so is a brand-new child of the dead meeting.
    let fresh = user_note(&s1, &w.m, "late note");
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![rec(&s1, "note", &fresh)])),
        vec![Tombstoned]
    );
}

#[test]
fn sync_tombstones_for_unknown_kinds_and_bad_gids_are_refused() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let good = common::gid(5, 1);
    let r = hub
        .store
        .apply_tombs(
            &s1.gid,
            &[
                SyncTombstone {
                    gid: good.clone(),
                    kind: "gizmo".into(),
                    lamport: 1,
                    origin: s1.gid.clone(),
                    cause: None,
                },
                SyncTombstone {
                    gid: "../etc".into(),
                    kind: "segment".into(),
                    lamport: 1,
                    origin: s1.gid.clone(),
                    cause: None,
                },
                tomb(&good, "note", &s1, None),
            ],
        )
        .unwrap();
    assert_eq!(r.rejected, vec![good.clone(), "../etc".to_string()]);
    // A tombstone for a gid with no row still blocks it.
    assert_eq!(r.applied, vec![good.clone()]);
    assert!(hub.store.is_tombstoned(&good).unwrap());
}

#[test]
fn sync_regenerate_keeps_an_edited_block_as_a_new_user_block() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let m = common::meeting(&s1.store, "Họp");
    let plain = note(&s1, &m, "AI plain", Provenance::Ai, false);
    let edited = note(&s1, &m, "AI edited here", Provenance::Ai, false);
    let pinned = note(&s1, &m, "AI pinned", Provenance::Ai, true);
    push(
        &hub,
        &s1,
        vec![
            rec(&s1, "meeting", &m),
            rec(&s1, "note", &plain),
            rec(&s1, "note", &edited),
            rec(&s1, "note", &pinned),
        ],
    );
    hub.store
        .update_note_block(&edited, "AI edited by the hub user")
        .unwrap();

    let regen = Some(TombCause::Regenerate);
    let r = hub
        .store
        .apply_tombs(
            &s1.gid,
            &[
                tomb(&plain, "note", &s1, regen),
                tomb(&edited, "note", &s1, regen),
                tomb(&pinned, "note", &s1, regen),
            ],
        )
        .unwrap();
    assert_eq!(r.applied.len(), 3);
    let blocks = hub.store.note_blocks(&m).unwrap();
    let mut texts: Vec<_> = blocks.iter().map(|b| b.body.as_str()).collect();
    texts.sort();
    assert_eq!(texts, vec!["AI edited by the hub user", "AI pinned"]);
    assert!(blocks.iter().all(|b| b.provenance == Provenance::User));
    assert!(
        blocks
            .iter()
            .all(|b| ![&plain, &edited, &pinned].contains(&&b.gid))
    );
    for g in [&plain, &edited, &pinned] {
        assert!(hub.store.is_tombstoned(g).unwrap());
    }

    // A user delete never leaves a copy.
    let n2 = user_note(&s1, &m, "user text");
    push(&hub, &s1, vec![rec(&s1, "note", &n2)]);
    hub.store
        .apply_tombs(&s1.gid, &[tomb(&n2, "note", &s1, Some(TombCause::User))])
        .unwrap();
    assert_eq!(hub.store.note_blocks(&m).unwrap().len(), 2);
}

// ------------------------------------------------------------- validation

#[test]
fn sync_one_bad_ciphertext_rejects_the_whole_batch_with_no_partial_writes() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    let mut batch = world_records(&s1, &w);
    // Seal the note under the wrong AAD (another gid's).
    let dek = dek_of(&batch[0]);
    let Record::Note(n) = &mut batch[3] else {
        unreachable!()
    };
    n.body_ct = Some(seal(
        &dek,
        "notes_blocks",
        "body_ct",
        &common::gid(7, 7),
        "x",
    ));
    let err = hub.store.apply_rows(&s1.gid, &batch).unwrap_err();
    match err {
        StoreError::BadRecord { gid, .. } => assert_eq!(gid, w.note),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        hub.store.get_meeting(&w.m),
        Err(StoreError::NotFound { .. })
    ));
    assert_eq!(hub.store.pending_count().unwrap(), 0);
    assert!(!hub.store.is_tombstoned(&w.m).unwrap());

    // A flipped byte, and a different key for a known meeting, are refused too.
    let mut good = world_records(&s1, &w);
    let Record::Segment(sg) = &mut good[2] else {
        unreachable!()
    };
    sg.text_ct.as_mut().unwrap().0[30] ^= 1;
    assert!(matches!(
        hub.store.apply_rows(&s1.gid, &good),
        Err(StoreError::BadRecord { .. })
    ));
    push(&hub, &s1, world_records(&s1, &w));
    let Record::Meeting(mut other_key) = rec(&s1, "meeting", &w.m) else {
        unreachable!()
    };
    other_key.dek = Some(Bytes(vec![9; 32]));
    other_key.version.lamport += 1;
    assert!(matches!(
        hub.store.apply_rows(&s1.gid, &[Record::Meeting(other_key)]),
        Err(StoreError::BadRecord { .. })
    ));
}

#[test]
fn sync_an_immutable_field_change_is_rejected() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let m = common::meeting(&s1.store, "Họp");
    push(&hub, &s1, vec![rec(&s1, "meeting", &m)]);
    type Tweak = fn(&mut ghi_store::sync::records::MeetingRec);
    let tweaks: [Tweak; 4] = [
        |r| r.started_at = Some(1),
        |r| r.source = Some("file".into()),
        |r| r.mode = Some("call".into()),
        |r| r.created_at = Some(r.created_at.unwrap_or(0) + 1),
    ];
    for tweak in tweaks {
        let Record::Meeting(mut r) = rec(&s1, "meeting", &m) else {
            unreachable!()
        };
        r.version.lamport += 1;
        tweak(&mut r);
        assert!(
            matches!(
                hub.store.apply_rows(&s1.gid, &[Record::Meeting(r)]),
                Err(StoreError::BadRecord { .. })
            ),
            "an immutable field changed"
        );
    }
    assert_eq!(
        hub.store.get_meeting(&m).unwrap().started_at,
        1_700_000_000_000
    );
}

#[test]
fn sync_batch_and_text_limits_are_enforced() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let m = common::meeting(&s1.store, "Họp");
    let one = rec(&s1, "meeting", &m);
    let many = vec![one.clone(); 257];
    assert!(matches!(
        hub.store.apply_rows(&s1.gid, &many),
        Err(StoreError::BadRecord { .. })
    ));
    let dek = dek_of(&one);
    let gid = common::gid(6, 1);
    let huge = Record::Note(ghi_store::sync::records::NoteRec {
        gid: gid.clone(),
        version: ver(3, &s1.gid),
        meeting_gid: m.clone(),
        kind: Some("paragraph".into()),
        provenance: Some("user".into()),
        body_ct: Some(seal(
            &dek,
            "notes_blocks",
            "body_ct",
            &gid,
            &"x".repeat(256 * 1024 + 1),
        )),
        ..Default::default()
    });
    assert!(matches!(
        hub.store.apply_rows(&s1.gid, &[one, huge]),
        Err(StoreError::BadRecord { .. })
    ));
    let bad_gid = Record::Mark(ghi_store::sync::records::MarkRec {
        gid: "not-a-uuid".into(),
        version: ver(1, &s1.gid),
        meeting_gid: m,
        ..Default::default()
    });
    assert!(matches!(
        hub.store.apply_rows(&s1.gid, &[bad_gid]),
        Err(StoreError::BadRecord { .. })
    ));
    assert!(matches!(
        hub.store.apply_rows(&common::gid(9, 99), &[]),
        Err(StoreError::NotFound { .. })
    ));
}

// ------------------------------------------------------ concurrent merges

/// Two devices edit the title of one meeting from the same version.
#[test]
fn sync_concurrent_title_edits_keep_the_loser_as_a_deterministic_copy() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let m = common::meeting(&s1.store, "Original");
    push(&hub, &s1, vec![rec(&s1, "meeting", &m)]);
    let l1 = lamport(&s1, "meeting", &m);
    s1.store.mark_clean(&m, l1).unwrap();
    assert!(!s1.store.sync_dirty("meeting", &m).unwrap());

    hub.store.set_meeting_title(&m, "Hub title").unwrap();
    s1.store.set_meeting_title(&m, "Spoke title").unwrap();
    let hub_ver = ver(lamport(&hub, "meeting", &m), &hub.gid);
    let spoke_rec = rec(&s1, "meeting", &m);
    let spoke_ver = spoke_rec.version().clone();
    assert_eq!(
        spoke_rec.version().lamport,
        hub_ver.lamport,
        "equal lamports: gids decide"
    );

    let r = push(&hub, &s1, vec![spoke_rec.clone()]);
    assert_eq!(outcomes(&r), vec![Merged]);
    let win = rules::winner(&hub_ver, &spoke_ver).clone();
    let (loser, win_title, lose_title) = if win == hub_ver {
        (&spoke_ver, "Hub title", "Spoke title")
    } else {
        (&hub_ver, "Spoke title", "Hub title")
    };
    assert_eq!(hub.store.get_meeting(&m).unwrap().title, win_title);
    let copies = hub.store.conflict_copies(&m).unwrap();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].text, lose_title);
    assert_eq!(copies[0].field, "title_ct");
    assert_eq!(copies[0].target_gid, m);
    assert_eq!(copies[0].origin, loser.origin);
    assert_eq!(
        copies[0].gid,
        rules::copy_gid(&m, "title_ct", loser.lamport, &loser.origin)
    );

    // Redelivery: the same copy, the same state.
    let before = state(&hub, "meeting", &[&m]);
    let r = push(&hub, &s1, vec![spoke_rec]);
    assert!(matches!(outcomes(&r)[0], Accepted | Merged));
    assert_eq!(hub.store.conflict_copies(&m).unwrap().len(), 1);
    assert_eq!(before, state(&hub, "meeting", &[&m]));

    // "Use this" writes the copy into the title and tombstones the copy.
    let gid = copies[0].gid.clone();
    hub.store.resolve_conflict(&gid, true).unwrap();
    assert_eq!(hub.store.get_meeting(&m).unwrap().title, lose_title);
    assert!(hub.store.conflict_copies(&m).unwrap().is_empty());
    assert!(hub.store.is_tombstoned(&gid).unwrap());
    // And a redelivery after the resolve does not bring that copy back.
    push(&hub, &s1, vec![rec(&s1, "meeting", &m)]);
    assert!(
        hub.store
            .conflict_copies(&m)
            .unwrap()
            .iter()
            .all(|c| c.gid != gid)
    );
}

/// Every free-text field makes a copy when it loses: forged edits from a
/// second spoke that never saw the row (no base), at a higher lamport.
#[test]
fn sync_free_text_fields_of_every_kind_make_copies() {
    let (hub, s1, c) = (node(), node(), node());
    link(&hub, &s1);
    link(&hub, &c);
    let w = world(&s1);
    push(&hub, &s1, world_records(&s1, &w));
    let dek = dek_of(&rec(&s1, "meeting", &w.m));

    // (record, loser text expected as copy)
    let Record::Segment(mut seg) = rec(&s1, "segment", &w.seg) else {
        unreachable!()
    };
    seg.version = ver(500, &c.gid);
    seg.base = None;
    seg.text_ct = Some(seal(&dek, "segments", "text_ct", &w.seg, "Câu mới từ C"));
    let Record::Note(mut nt) = rec(&s1, "note", &w.note) else {
        unreachable!()
    };
    nt.version = ver(500, &c.gid);
    nt.base = None;
    nt.body_ct = Some(seal(
        &dek,
        "notes_blocks",
        "body_ct",
        &w.note,
        "Ghi chú từ C",
    ));
    let Record::ActionItem(mut ai) = rec(&s1, "action_item", &w.action) else {
        unreachable!()
    };
    ai.version = ver(500, &c.gid);
    ai.base = None;
    ai.text_ct = Some(seal(
        &dek,
        "action_items",
        "text_ct",
        &w.action,
        "Việc từ C",
    ));
    ai.due_text_ct = Some(seal(
        &dek,
        "action_items",
        "due_text_ct",
        &w.action,
        "thứ hai",
    ));
    let Record::Speaker(mut sp) = rec(&s1, "speaker", &w.speaker) else {
        unreachable!()
    };
    sp.version = ver(500, &c.gid);
    sp.base = None;
    sp.display_name_ct = Some(seal(
        &dek,
        "speakers",
        "display_name_ct",
        &w.speaker,
        "Bình",
    ));

    let r = hub
        .store
        .apply_rows(
            &c.gid,
            &[
                Record::Segment(seg),
                Record::Note(nt),
                Record::ActionItem(ai),
                Record::Speaker(sp),
            ],
        )
        .unwrap();
    assert_eq!(outcomes(&r), vec![Merged; 4]);
    // The new values won (lamport 500) ...
    assert_eq!(hub.store.segments(&w.m).unwrap()[0].text, "Câu mới từ C");
    assert_eq!(bodies(&hub, &w.m), vec!["Ghi chú từ C"]);
    let item = &hub.store.action_items(&w.m).unwrap()[0];
    assert_eq!(
        (item.text.as_str(), item.due_text.as_deref()),
        ("Việc từ C", Some("thứ hai"))
    );
    // ... and the old ones are kept as copies.
    let mut copies: Vec<_> = hub
        .store
        .conflict_copies(&w.m)
        .unwrap()
        .into_iter()
        .map(|c| (c.target_kind, c.field, c.text))
        .collect();
    copies.sort();
    let mut want = vec![
        (
            "action_item".to_string(),
            "due_text_ct".to_string(),
            "thứ sáu".to_string(),
        ),
        (
            "action_item".to_string(),
            "text_ct".to_string(),
            "Gửi báo cáo".to_string(),
        ),
        (
            "note".to_string(),
            "body_ct".to_string(),
            "Ghi chú đầu tiên".to_string(),
        ),
        (
            "segment".to_string(),
            "text_ct".to_string(),
            "Chốt kế hoạch quý bốn".to_string(),
        ),
        (
            "speaker".to_string(),
            "display_name_ct".to_string(),
            "Linh".to_string(),
        ),
    ];
    want.sort();
    assert_eq!(copies, want);
    // The segment's FTS row follows the winner.
    assert!(
        !hub.store
            .search(&SearchQuery::new("cau moi"))
            .unwrap()
            .is_empty()
    );
    assert!(
        hub.store
            .search(&SearchQuery::new("chot ke hoach"))
            .unwrap()
            .is_empty()
    );
}

/// A text that loses (lower version) is also kept.
#[test]
fn sync_the_losing_record_keeps_the_stored_row_and_copies_the_record_text() {
    let (hub, s1, c) = (node(), node(), node());
    link(&hub, &s1);
    link(&hub, &c);
    let m = common::meeting(&s1.store, "Họp");
    let n = user_note(&s1, &m, "from S1");
    push(
        &hub,
        &s1,
        vec![rec(&s1, "meeting", &m), rec(&s1, "note", &n)],
    );
    let dek = dek_of(&rec(&s1, "meeting", &m));
    let Record::Note(mut low) = rec(&s1, "note", &n) else {
        unreachable!()
    };
    low.version = ver(1, &c.gid);
    low.base = None;
    low.pinned = Some(true);
    low.body_ct = Some(seal(&dek, "notes_blocks", "body_ct", &n, "old from C"));
    let r = push(&hub, &c, vec![Record::Note(low)]);
    assert_eq!(outcomes(&r), vec![Merged]);
    assert_eq!(bodies(&hub, &m), vec!["from S1"]);
    assert!(
        !hub.store.note_blocks(&m).unwrap()[0].pinned,
        "LWW fields stay with the winner"
    );
    let copies = hub.store.conflict_copies(&m).unwrap();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].text, "old from C");
    assert_eq!(copies[0].origin, c.gid);
}

/// Two spokes edit at equal Lamport values; a stale base from the third must
/// not overwrite the first one's text (the hub compares exact versions).
#[test]
fn sync_a_stale_base_with_an_equal_lamport_is_concurrent_not_a_take() {
    let (hub, a, c) = (node(), node(), node());
    link(&hub, &a);
    link(&hub, &c);
    let m = common::meeting(&a.store, "Họp");
    let n = user_note(&a, &m, "base text");
    push(&hub, &a, vec![rec(&a, "meeting", &m), rec(&a, "note", &n)]);
    let dek = dek_of(&rec(&a, "meeting", &m));

    // C's edit lands first and the hub row is (50, C).
    let Record::Note(mut from_c) = rec(&a, "note", &n) else {
        unreachable!()
    };
    from_c.base = Some(from_c.version.clone());
    from_c.version = ver(50, &c.gid);
    from_c.body_ct = Some(seal(&dek, "notes_blocks", "body_ct", &n, "from C"));
    assert_eq!(
        outcomes(&push(&hub, &c, vec![Record::Note(from_c)])),
        vec![Accepted]
    );
    assert_eq!(bodies(&hub, &m), vec!["from C"]);

    // A's edit was based on "(50, hub)": same Lamport, other writer.
    let Record::Note(mut from_a) = rec(&a, "note", &n) else {
        unreachable!()
    };
    from_a.base = Some(ver(50, &hub.gid));
    from_a.version = ver(51, &a.gid);
    from_a.body_ct = Some(seal(&dek, "notes_blocks", "body_ct", &n, "from A"));
    assert_eq!(
        outcomes(&push(&hub, &a, vec![Record::Note(from_a)])),
        vec![Merged]
    );
    assert_eq!(bodies(&hub, &m), vec!["from A"]);
    let copies = hub.store.conflict_copies(&m).unwrap();
    assert_eq!(copies.len(), 1, "C's text is kept");
    assert_eq!(copies[0].text, "from C");
    assert_eq!(copies[0].origin, c.gid);
}

#[test]
fn sync_a_writer_the_receiver_has_no_pairing_with_is_a_known_device() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let m = common::meeting(&s1.store, "Họp");
    let n = user_note(&s1, &m, "text");
    push(
        &hub,
        &s1,
        vec![rec(&s1, "meeting", &m), rec(&s1, "note", &n)],
    );
    let stranger = common::gid(9, 200);
    let Record::Note(mut rel) = rec(&s1, "note", &n) else {
        unreachable!()
    };
    rel.version = ver(60, &stranger);
    rel.base = None;
    let relayed = Record::Note(rel);
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![relayed.clone()])),
        vec![Merged]
    );
    // The row names its true writer, and a known device is not a pairing.
    assert_eq!(rec(&hub, "note", &n).version().origin, stranger);
    assert!(hub.store.device(&stranger).unwrap().is_none());
    assert!(
        hub.store
            .devices()
            .unwrap()
            .iter()
            .all(|d| d.gid != stranger)
    );
    assert!(hub.store.device_by_key(&[0; 32]).unwrap().is_none());
    // Redelivery is the identical version.
    assert_eq!(outcomes(&push(&hub, &s1, vec![relayed])), vec![Accepted]);
    assert_eq!(hub.store.conflict_copies(&m).unwrap().len(), 0);
    // Pairing with it later keeps its id (the row's origin stays right).
    let late = Node {
        gid: stranger.clone(),
        ..node()
    };
    hub.store
        .pin_device(&new_device(&late, DeviceRole::Spoke), &[3; 32])
        .unwrap();
    assert_eq!(rec(&hub, "note", &n).version().origin, stranger);
    assert!(hub.store.device(&stranger).unwrap().is_some());
    // Unpairing keeps the name for the origin column but hides it again.
    hub.store.unpin_device(&stranger).unwrap();
    assert!(hub.store.device(&stranger).unwrap().is_none());
    assert_eq!(rec(&hub, "note", &n).version().origin, stranger);
}

// -------------------------------------------------------------- field rules

#[test]
fn sync_monotone_meeting_fields_merge_and_lww_fields_follow_the_winner() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let m = common::meeting(&s1.store, "Họp");
    push(&hub, &s1, vec![rec(&s1, "meeting", &m)]);
    hub.store.set_meeting_status(&m, "processing").unwrap();
    hub.store.extend_meeting_duration(&m, 9_000).unwrap();
    hub.store.set_consent_confirmed(&m, true).unwrap();

    // From the spoke: lower status and duration, consent unset, cloud used,
    // higher epochs, new LWW values at a higher lamport.
    let Record::Meeting(mut r) = rec(&s1, "meeting", &m) else {
        unreachable!()
    };
    r.dek = None;
    r.version.lamport += 100;
    r.status = Some("importing".into());
    r.duration_ms = Some(2_000);
    r.consent_confirmed = Some(false);
    r.cloud_used = Some(true);
    r.transcript_version = Some(2);
    r.transcript_epoch = Some(4);
    r.ai_epoch = Some(3);
    r.lang = Some("vi".into());
    r.sensitive = Some(true);
    r.template = Some("tpl".into());
    let out = push(&hub, &s1, vec![Record::Meeting(r)]);
    assert_eq!(outcomes(&out), vec![Accepted]);
    let Record::Meeting(got) = rec(&hub, "meeting", &m) else {
        unreachable!()
    };
    assert_eq!(got.status.as_deref(), Some("processing"), "rank max");
    assert_eq!(got.duration_ms, Some(9_000), "duration max");
    assert_eq!(got.consent_confirmed, Some(true), "OR");
    assert_eq!(got.cloud_used, Some(true), "OR");
    assert_eq!(
        (got.transcript_version, got.transcript_epoch),
        (Some(2), Some(4))
    );
    assert_eq!(got.ai_epoch, Some(3));
    assert_eq!(got.lang.as_deref(), Some("vi"));
    assert_eq!(got.sensitive, Some(true));
    assert_eq!(got.template.as_deref(), Some("tpl"));

    // A concurrent record that loses keeps the group, but monotone fields
    // still merge.
    let Record::Meeting(mut low) = rec(&s1, "meeting", &m) else {
        unreachable!()
    };
    low.dek = None;
    low.version = ver(1, &s1.gid);
    low.base = None;
    low.lang = Some("en".into());
    low.status = Some("ready".into());
    low.duration_ms = Some(20_000);
    let out = push(&hub, &s1, vec![Record::Meeting(low)]);
    assert_eq!(outcomes(&out), vec![Merged]);
    let Record::Meeting(got) = rec(&hub, "meeting", &m) else {
        unreachable!()
    };
    assert_eq!(got.lang.as_deref(), Some("vi"));
    assert_eq!(got.status.as_deref(), Some("ready"));
    assert_eq!(got.duration_ms, Some(20_000));
}

#[test]
fn sync_persons_are_last_writer_wins_without_copies_and_never_become_me() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let p = s1.store.add_person("Linh", 2).unwrap();
    push(&hub, &s1, vec![rec(&s1, "person", &p)]);
    let Record::Person(mut r) = rec(&s1, "person", &p) else {
        unreachable!()
    };
    r.version.lamport += 5;
    r.name = Some("Linh Nguyễn".into());
    r.color_slot = Some(4);
    r.is_me = Some(true);
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![Record::Person(r)])),
        vec![Accepted]
    );
    let Record::Person(got) = rec(&hub, "person", &p) else {
        unreachable!()
    };
    assert_eq!(got.name.as_deref(), Some("Linh Nguyễn"));
    assert_eq!(got.color_slot, Some(4));
    assert_eq!(got.is_me, Some(false));
    assert_ne!(hub.store.me_person().unwrap(), p);

    // The same name made on both devices: two rows, no failure (the collision
    // merge is a later slice).
    let local = hub.store.add_person("Bình", 1).unwrap();
    let remote = s1.store.add_person("Bình", 1).unwrap();
    assert_ne!(local, remote);
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![rec(&s1, "person", &remote)])),
        vec![Accepted]
    );
    assert!(
        hub.store
            .encode_record("person", &remote, false)
            .unwrap()
            .is_some()
    );
}

#[test]
fn sync_marks_are_insert_only_and_tracks_take_the_smaller_cut() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    push(&hub, &s1, world_records(&s1, &w));
    let Record::Mark(mut mk) = rec(&s1, "mark", &w.mark) else {
        unreachable!()
    };
    mk.version.lamport += 10;
    mk.t_ms = Some(9_999);
    mk.tag = Some("decision".into());
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![Record::Mark(mk)])),
        vec![Accepted]
    );
    let marks = hub.store.marks(&w.m).unwrap();
    assert_eq!((marks[0].t_ms, marks[0].tag), (500, MarkTag::Star));

    let tg = common::gid(2, 1);
    let track = |v: i64, pages: i64, cut: Option<i64>, kind: &str| {
        Record::Track(TrackRec {
            gid: tg.clone(),
            version: ver(v, &s1.gid),
            meeting_gid: w.m.clone(),
            kind: Some(kind.into()),
            page_count: Some(pages),
            cut_pages: cut,
            ..Default::default()
        })
    };
    push(&hub, &s1, vec![track(100, 10, None, "mic")]);
    push(&hub, &s1, vec![track(101, 10, Some(4), "mic")]);
    push(&hub, &s1, vec![track(102, 10, Some(8), "mic")]);
    let Record::Track(t) = rec(&hub, "track", &tg) else {
        unreachable!()
    };
    assert_eq!(t.cut_pages, Some(4));
    assert_eq!(t.page_count, Some(4), "the page count follows the cut");
    assert!(matches!(
        hub.store
            .apply_rows(&s1.gid, &[track(103, 4, Some(4), "system")]),
        Err(StoreError::BadRecord { .. })
    ));
}

#[test]
fn sync_folders_tags_and_links_arrive_in_any_order() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let m = common::meeting(&s1.store, "Họp");
    let folder = s1.store.create_folder("Dự án").unwrap().gid;
    let tag = s1.store.create_tag("Quan trọng").unwrap().gid;
    s1.store
        .set_meeting_folder(std::slice::from_ref(&m), Some(&folder))
        .unwrap();
    s1.store
        .tag_meetings(std::slice::from_ref(&m), &tag)
        .unwrap();
    let Record::MeetingTag(lk) = rec(&s1, "meeting_tag", &link_gids(&s1).pop().expect("a link"))
    else {
        unreachable!()
    };

    // The link and the meeting before their tag and folder.
    let r = push(
        &hub,
        &s1,
        vec![Record::MeetingTag(lk.clone()), rec(&s1, "meeting", &m)],
    );
    assert_eq!(outcomes(&r), vec![Parked, Accepted]);
    assert_eq!(hub.store.get_meeting(&m).unwrap().folder_gid, None);
    assert_eq!(
        hub.store.pending_count().unwrap(),
        2,
        "the link and the folder stub"
    );
    push(
        &hub,
        &s1,
        vec![rec(&s1, "tag", &tag), rec(&s1, "folder", &folder)],
    );
    assert_eq!(hub.store.pending_count().unwrap(), 0);
    assert_eq!(
        hub.store.get_meeting(&m).unwrap().folder_gid,
        Some(folder.clone())
    );
    let tags = hub.store.meeting_tags(std::slice::from_ref(&m)).unwrap();
    assert_eq!(tags[&m].len(), 1);

    // The same tag added on two devices: the lower link gid stays.
    let low = "00000000-0000-7000-8000-000000000001".to_string();
    let high = "ffffffff-ffff-7fff-8fff-ffffffffffff".to_string();
    let mk = |gid: &str, lamport: i64| {
        Record::MeetingTag(MeetingTagRec {
            gid: gid.into(),
            version: ver(lamport, &s1.gid),
            meeting_gid: m.clone(),
            tag_gid: tag.clone(),
            ..Default::default()
        })
    };
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![mk(&high, 900)])),
        vec![Tombstoned]
    );
    assert!(hub.store.is_tombstoned(&high).unwrap());
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![mk(&low, 901)])),
        vec![Accepted]
    );
    assert!(hub.store.is_tombstoned(&lk.gid).unwrap());
    assert!(
        hub.store
            .encode_record("meeting_tag", &low, false)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        hub.store.meeting_tags(std::slice::from_ref(&m)).unwrap()[&m].len(),
        1
    );

    // A tag that was deleted drops a link that names it.
    hub.store
        .apply_tombs(&s1.gid, &[tomb(&tag, "tag", &s1, Some(TombCause::User))])
        .unwrap();
    let again = mk(&common::gid(11, 5), 1_000);
    assert_eq!(outcomes(&push(&hub, &s1, vec![again])), vec![Tombstoned]);
}

#[test]
fn sync_conflict_copies_are_a_synced_row_kind() {
    let (hub, s1, spoke2) = (node(), node(), node());
    link(&hub, &s1);
    link(&hub, &spoke2);
    let m = common::meeting(&s1.store, "Họp");
    push(&hub, &s1, vec![rec(&s1, "meeting", &m)]);
    let dek = dek_of(&rec(&s1, "meeting", &m));
    let cg = common::gid(13, 1);
    let copy = Record::ConflictCopy(ConflictCopyRec {
        gid: cg.clone(),
        version: ver(70, &s1.gid),
        meeting_gid: m.clone(),
        target_kind: "meeting".into(),
        target_gid: m.clone(),
        field: "title_ct".into(),
        value_ct: Some(seal(
            &dek,
            "conflict_copies",
            "value_ct",
            &cg,
            "Tiêu đề thua",
        )),
        created_at: Some(1_700_000_000_000),
        ..Default::default()
    });
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![copy.clone()])),
        vec![Accepted]
    );
    assert_eq!(
        hub.store.conflict_copies(&m).unwrap()[0].text,
        "Tiêu đề thua"
    );
    assert_eq!(rec(&hub, "conflict_copy", &cg), copy);
    // A copy that does not open, or that points nowhere, is refused.
    let Record::ConflictCopy(mut bad) = copy.clone() else {
        unreachable!()
    };
    bad.gid = common::gid(13, 2);
    assert!(matches!(
        hub.store.apply_rows(&s1.gid, &[Record::ConflictCopy(bad)]),
        Err(StoreError::BadRecord { .. })
    ));
    let Record::ConflictCopy(mut bad) = copy else {
        unreachable!()
    };
    bad.field = "kind".into();
    assert!(matches!(
        hub.store.apply_rows(&s1.gid, &[Record::ConflictCopy(bad)]),
        Err(StoreError::BadRecord { .. })
    ));
    // Dismissing tombstones it, and it stays gone.
    hub.store.resolve_conflict(&cg, false).unwrap();
    assert!(hub.store.conflict_copies(&m).unwrap().is_empty());
    assert_eq!(hub.store.get_meeting(&m).unwrap().title, "Họp");
}

#[test]
fn sync_synced_settings_go_to_the_settings_layer() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let key = "meetingLanguage";
    let gid = ghi_store::sync::settings::setting_gid(key);
    let set = |l: i64, v: &str| {
        Record::Setting(SettingRec {
            gid: gid.clone(),
            version: ver(l, &s1.gid),
            key: key.into(),
            value_json: Some(v.into()),
            ..Default::default()
        })
    };
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![set(40, "\"vi\"")])),
        vec![Accepted]
    );
    let app = hub.store.get_setting("app").unwrap().unwrap();
    assert_eq!(app["meetingLanguage"], "vi");
    // An unknown key and a value of the wrong shape are dropped, not fatal.
    push(&hub, &s1, vec![set(41, "\"klingon\"")]);
    let other = Record::Setting(SettingRec {
        gid: common::gid(14, 1),
        version: ver(42, &s1.gid),
        key: "shortcuts".into(),
        value_json: Some("1".into()),
        ..Default::default()
    });
    push(&hub, &s1, vec![other]);
    assert_eq!(
        hub.store.get_setting("app").unwrap().unwrap()["meetingLanguage"],
        "vi"
    );
}

// ------------------------------------------------------------ epochs, parking

fn bump(r: &mut ghi_store::sync::records::MeetingRec, lamport: i64) {
    r.version.lamport += lamport;
}

#[test]
fn sync_a_v2_segment_before_its_meeting_is_parked_then_applied() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    let Record::Segment(mut v2) = rec(&s1, "segment", &w.seg) else {
        unreachable!()
    };
    v2.speaker_gid = None;
    v2.transcript_version = Some(2);
    v2.epoch = Some(1);
    let Record::Meeting(mut m2) = rec(&s1, "meeting", &w.m) else {
        unreachable!()
    };
    m2.transcript_version = Some(2);
    m2.transcript_epoch = Some(1);

    // The segment is first: its meeting is missing.
    let r = push(&hub, &s1, vec![Record::Segment(v2.clone())]);
    assert_eq!(outcomes(&r), vec![Parked]);
    assert_eq!(hub.store.pending_count().unwrap(), 1);
    // The meeting arrives (in a later batch): the segment is applied.
    let r = push(&hub, &s1, vec![Record::Meeting(m2.clone())]);
    assert_eq!(outcomes(&r), vec![Accepted]);
    assert_eq!(hub.store.pending_count().unwrap(), 0);
    assert_eq!(hub.store.get_meeting(&w.m).unwrap().transcript_version, 2);
    assert_eq!(
        hub.store.segments(&w.m).unwrap()[0].text,
        "Chốt kế hoạch quý bốn"
    );

    // Same when the meeting exists but is older: a greater (version, epoch)
    // is held, not dropped, until the meeting catches up.
    let hub2 = node();
    link(&hub2, &s1);
    let Record::Meeting(mut m1) = rec(&s1, "meeting", &w.m) else {
        unreachable!()
    };
    m1.transcript_version = Some(1);
    m1.transcript_epoch = Some(0);
    push(&hub2, &s1, vec![Record::Meeting(m1)]);
    assert_eq!(
        outcomes(&push(&hub2, &s1, vec![Record::Segment(v2.clone())])),
        vec![Parked]
    );
    assert!(!hub2.store.is_tombstoned(&w.seg).unwrap());
    bump(&mut m2, 10);
    push(&hub2, &s1, vec![Record::Meeting(m2.clone())]);
    assert_eq!(hub2.store.pending_count().unwrap(), 0);
    assert_eq!(hub2.store.segments(&w.m).unwrap().len(), 1);

    // A lower generation is superseded: dropped and tombstoned.
    let hub3 = node();
    link(&hub3, &s1);
    push(&hub3, &s1, vec![Record::Meeting(m2)]);
    let Record::Segment(old) = rec(&s1, "segment", &w.seg) else {
        unreachable!()
    };
    assert_eq!(
        outcomes(&push(&hub3, &s1, vec![Record::Segment(old)])),
        vec![Tombstoned]
    );
    assert!(hub3.store.is_tombstoned(&w.seg).unwrap());
    assert!(hub3.store.segments(&w.m).unwrap().is_empty());
}

#[test]
fn sync_ai_blocks_follow_the_ai_epoch() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let m = common::meeting(&s1.store, "Họp");
    let ai = note(&s1, &m, "AI", Provenance::Ai, false);
    let pinned = note(&s1, &m, "AI pinned", Provenance::Ai, true);
    let forge = |gid: &str, epoch: i64| {
        let Record::Note(mut r) = rec(&s1, "note", gid) else {
            unreachable!()
        };
        r.epoch = Some(epoch);
        Record::Note(r)
    };
    let Record::Meeting(mut at2) = rec(&s1, "meeting", &m) else {
        unreachable!()
    };
    at2.ai_epoch = Some(2);

    push(&hub, &s1, vec![rec(&s1, "meeting", &m)]);
    // Newer than the meeting's: parked (even a pinned block is not exempt
    // from being newer, but exempt from being dropped).
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![forge(&ai, 2)])),
        vec![Parked]
    );
    bump(&mut at2, 5);
    push(&hub, &s1, vec![Record::Meeting(at2)]);
    assert_eq!(hub.store.pending_count().unwrap(), 0);
    assert_eq!(bodies(&hub, &m), vec!["AI"]);

    // Older than the meeting's: superseded; pinned blocks stay.
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![forge(&pinned, 1)])),
        vec![Accepted]
    );
    let stale = note(&s1, &m, "stale AI", Provenance::Ai, false);
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![forge(&stale, 1)])),
        vec![Tombstoned]
    );
    assert!(hub.store.is_tombstoned(&stale).unwrap());
    assert_eq!(hub.store.note_blocks(&m).unwrap().len(), 2);
}

#[test]
fn sync_an_orphan_is_parked_until_its_parent_arrives() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    // Children first, in one batch with the meeting last.
    let mut batch = world_records(&s1, &w);
    batch.rotate_left(1);
    let r = push(&hub, &s1, batch);
    // The speaker still comes before the segment that names it; only the
    // meeting's own children were early.
    assert_eq!(
        outcomes(&r),
        vec![Parked, Parked, Parked, Parked, Parked, Accepted]
    );
    assert_eq!(
        hub.store.pending_count().unwrap(),
        0,
        "retried after the same batch"
    );
    assert_eq!(hub.store.segments(&w.m).unwrap().len(), 1);
    assert_eq!(bodies(&hub, &w.m), vec!["Ghi chú đầu tiên"]);
    assert_eq!(hub.store.speakers(&w.m).unwrap().len(), 1);
    assert_eq!(
        hub.store.segments(&w.m).unwrap()[0].speaker_gid.as_deref(),
        Some(w.speaker.as_str())
    );
}

#[test]
fn sync_a_parked_orphan_is_dropped_when_its_parent_is_tombstoned() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    let r = push(
        &hub,
        &s1,
        vec![rec(&s1, "note", &w.note), rec(&s1, "segment", &w.seg)],
    );
    assert_eq!(outcomes(&r), vec![Parked, Parked]);
    assert_eq!(hub.store.pending_count().unwrap(), 2);
    hub.store
        .apply_tombs(
            &s1.gid,
            &[tomb(&w.m, "meeting", &s1, Some(TombCause::User))],
        )
        .unwrap();
    assert_eq!(hub.store.pending_count().unwrap(), 0);
    // The meeting never arrives afterwards.
    assert_eq!(
        outcomes(&push(&hub, &s1, vec![rec(&s1, "meeting", &w.m)])),
        vec![Tombstoned]
    );
}

#[test]
fn sync_parked_records_expire_after_seven_days() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    push(&hub, &s1, vec![rec(&s1, "note", &w.note)]);
    assert_eq!(hub.store.pending_count().unwrap(), 1);
    assert_eq!(hub.store.retry_pending().unwrap(), 0);
    assert_eq!(hub.store.pending_count().unwrap(), 1);
    // Age it past the limit.
    let old = ghi_store::store::now_ms() - ghi_store::sync::pending::MAX_PENDING_AGE_MS - 1;
    hub.raw()
        .execute("UPDATE sync_pending SET received_at = ?1", [old])
        .unwrap();
    assert_eq!(hub.store.retry_pending().unwrap(), 0);
    assert_eq!(hub.store.pending_count().unwrap(), 0);
}

// -------------------------------------------------------------- spoke side

#[test]
fn sync_a_spoke_keeps_dirty_rows_on_pull_and_takes_the_hubs_clean_ones() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let m = common::meeting(&s1.store, "Gốc");
    // Push, ack.
    push(&hub, &s1, vec![rec(&s1, "meeting", &m)]);
    assert!(
        s1.store.sync_dirty("meeting", &m).unwrap(),
        "never acked: dirty"
    );
    s1.store
        .mark_clean(&m, lamport(&s1, "meeting", &m))
        .unwrap();
    assert!(!s1.store.sync_dirty("meeting", &m).unwrap());
    // A stale ack does not clean a row that moved on.
    s1.store.set_meeting_title(&m, "Sửa ở điện thoại").unwrap();
    assert!(s1.store.sync_dirty("meeting", &m).unwrap());
    s1.store.mark_clean(&m, 1).unwrap();
    assert!(s1.store.sync_dirty("meeting", &m).unwrap());

    // The hub changed it too; the spoke pulls: its own edit is kept.
    hub.store.set_meeting_title(&m, "Sửa ở máy tính").unwrap();
    let pulled = hub
        .store
        .encode_record("meeting", &m, true)
        .unwrap()
        .unwrap();
    let r = s1
        .store
        .apply_rows(&hub.gid, std::slice::from_ref(&pulled))
        .unwrap();
    assert_eq!(outcomes(&r), vec![Merged]);
    assert_eq!(s1.store.get_meeting(&m).unwrap().title, "Sửa ở điện thoại");
    assert!(s1.store.sync_dirty("meeting", &m).unwrap());

    // After the push is acked the row is clean and the hub's version is taken.
    s1.store
        .mark_clean(&m, lamport(&s1, "meeting", &m))
        .unwrap();
    let r = s1.store.apply_rows(&hub.gid, &[pulled]).unwrap();
    assert_eq!(outcomes(&r), vec![Accepted]);
    assert_eq!(s1.store.get_meeting(&m).unwrap().title, "Sửa ở máy tính");
    assert!(
        !s1.store.sync_dirty("meeting", &m).unwrap(),
        "base follows the hub"
    );
    // The pulled row is not pushed back as an edit: same version, no change.
    let again = hub
        .store
        .encode_record("meeting", &m, false)
        .unwrap()
        .unwrap();
    assert_eq!(
        outcomes(&s1.store.apply_rows(&hub.gid, &[again]).unwrap()),
        vec![Accepted]
    );
    assert!(!s1.store.sync_dirty("meeting", &m).unwrap());
}

#[test]
fn sync_a_spoke_pulling_a_new_meeting_stores_it_clean() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&hub);
    let recs = world_records(&hub, &w);
    let r = s1.store.apply_rows(&hub.gid, &recs).unwrap();
    assert_eq!(outcomes(&r), vec![Accepted; 6]);
    for (kind, gid) in [
        ("meeting", &w.m),
        ("speaker", &w.speaker),
        ("segment", &w.seg),
        ("note", &w.note),
        ("action_item", &w.action),
        ("mark", &w.mark),
    ] {
        assert!(!s1.store.sync_dirty(kind, gid).unwrap(), "{kind} is clean");
    }
    assert_eq!(s1.store.segments(&w.m).unwrap().len(), 1);
    // The spoke edits: only that row is dirty, and it carries its base.
    s1.store.update_note_block(&w.note, "Sửa").unwrap();
    assert!(s1.store.sync_dirty("note", &w.note).unwrap());
    let Record::Note(n) = rec(&s1, "note", &w.note) else {
        unreachable!()
    };
    assert_eq!(
        n.base.as_ref().map(|b| b.origin.as_str()),
        Some(hub.gid.as_str())
    );
    assert_eq!(n.version.origin, s1.gid);
    // The hub takes it (base = the hub's own version): no conflict copy.
    let out = push(&hub, &s1, vec![Record::Note(n)]);
    assert_eq!(outcomes(&out), vec![Accepted]);
    assert_eq!(bodies(&hub, &w.m), vec!["Sửa"]);
    assert!(hub.store.conflict_copies(&w.m).unwrap().is_empty());
}

#[test]
fn sync_a_smaller_cut_truncates_the_local_bundle() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let m = common::meeting(&hub.store, "Họp");
    let mut w = hub
        .store
        .open_track(&m, ghi_store::store::TrackKind::Mic)
        .unwrap();
    for i in 0..6u8 {
        w.append(&[i; 64]).unwrap();
    }
    hub.store
        .finish_track(&m, ghi_store::store::TrackKind::Mic, w)
        .unwrap();
    let tg: String = hub
        .raw()
        .query_row("SELECT gid FROM tracks", [], |r| r.get(0))
        .unwrap();
    // The spoke's meeting row and key first (the hub already has this one).
    let Record::Track(mut t) = rec(&hub, "track", &tg) else {
        unreachable!()
    };
    t.version = ver(t.version.lamport + 50, &s1.gid);
    t.base = None;
    t.cut_pages = Some(3);
    t.page_count = Some(6);
    let r = push(&hub, &s1, vec![Record::Track(t)]);
    assert_eq!(outcomes(&r), vec![Merged]);
    assert_eq!(
        hub.store.tracks(&m).unwrap(),
        [(ghi_store::store::TrackKind::Mic, 3)]
    );
    let reader = hub
        .store
        .open_bundle(&m, ghi_store::store::TrackKind::Mic)
        .unwrap();
    assert_eq!(reader.page_count(), 3);
    assert_eq!(reader.page(2).unwrap(), vec![2u8; 64]);
    let Record::Track(back) = rec(&hub, "track", &tg) else {
        unreachable!()
    };
    assert_eq!(back.cut_pages, Some(3));
    // The cut was this device's own act: its version is ours again.
    assert_eq!(back.version.origin, hub.gid);

    // A tombstone (retention) removes the bundle.
    hub.store
        .apply_tombs(
            &s1.gid,
            &[tomb(&tg, "track", &s1, Some(TombCause::Retention))],
        )
        .unwrap();
    assert!(hub.store.tracks(&m).unwrap().is_empty());
    assert!(
        !hub.store
            .bundle_path(&m, ghi_store::store::TrackKind::Mic)
            .unwrap()
            .exists()
    );
}

#[test]
fn sync_a_local_edit_takes_the_version_back_from_the_peer() {
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    push(&hub, &s1, world_records(&s1, &w));
    assert_eq!(rec(&hub, "note", &w.note).version().origin, s1.gid);
    hub.store.update_note_block(&w.note, "Hub edit").unwrap();
    assert_eq!(rec(&hub, "note", &w.note).version().origin, hub.gid);
    // The spoke's next push (based on its own acked version) now conflicts.
    s1.store
        .mark_clean(&w.note, lamport(&s1, "note", &w.note))
        .unwrap();
    s1.store.update_note_block(&w.note, "Spoke edit").unwrap();
    let r = push(&hub, &s1, vec![rec(&s1, "note", &w.note)]);
    assert_eq!(outcomes(&r), vec![Merged]);
    assert_eq!(hub.store.conflict_copies(&w.m).unwrap().len(), 1);
}

// ------------------------------------------------- 15-C2b: deferred merge rules

/// One direction of a sync session: everything `from` has (tombstones first)
/// goes to `to`, as `from` would send it. Sending all of it again is what a
/// repeated session amounts to: it must change nothing.
fn flow(from: &Node, to: &Node) {
    flow_as(from, to, false);
}

/// `ack`: `from` is a spoke whose rows count as pushed (clean) afterwards.
fn flow_as(from: &Node, to: &Node, ack: bool) {
    let t = from.store.tombs_since(0, 100_000).unwrap();
    to.store.apply_tombs(&from.gid, &t.tombs).unwrap();
    let mut seq = 0;
    loop {
        let b = from.store.changes_since(seq, 256).unwrap();
        // The feed leaves a meeting's key out (it travels on its own).
        let recs: Vec<Record> = b
            .changes
            .into_iter()
            .map(|c| match c.record {
                Record::Meeting(m) => rec(from, "meeting", &m.gid),
                r => r,
            })
            .collect();
        to.store.apply_rows(&from.gid, &recs).unwrap();
        if ack {
            for r in &recs {
                from.store.mark_clean(r.gid(), r.version().lamport).unwrap();
            }
        }
        seq = b.upto_seq;
        if !b.more {
            break;
        }
    }
}

/// A full round: spokes push, the hub merges, spokes pull.
fn round(hub: &Node, spokes: &[&Node], order: &[usize]) {
    for &i in order {
        flow_as(spokes[i], hub, true);
    }
    for &i in order {
        flow(hub, spokes[i]);
    }
}

/// What the user sees of organisation, plus the tombstones.
#[derive(Debug, PartialEq)]
struct Org {
    folders: Vec<(String, String)>,
    tags: Vec<(String, String)>,
    meetings: Vec<(String, Option<String>, Vec<String>)>,
}

fn org(n: &Node, ms: &[String]) -> Org {
    let tags = n.store.meeting_tags(ms).unwrap();
    let mut meetings: Vec<_> = ms
        .iter()
        .map(|m| {
            let mut t: Vec<String> = tags
                .get(m)
                .map(|v| v.iter().map(|t| t.gid.clone()).collect())
                .unwrap_or_default();
            t.sort();
            (m.clone(), n.store.get_meeting(m).unwrap().folder_gid, t)
        })
        .collect();
    meetings.sort();
    Org {
        folders: n
            .store
            .folders()
            .unwrap()
            .into_iter()
            .map(|f| (f.gid, f.name))
            .collect(),
        tags: n
            .store
            .tags()
            .unwrap()
            .into_iter()
            .map(|t| (t.gid, t.name))
            .collect(),
        meetings,
    }
}

fn done_meeting(n: &Node, title: &str) -> String {
    let m = common::meeting(&n.store, title);
    n.store.finish_meeting(&m, 1_000).unwrap();
    m
}

#[test]
fn sync_same_name_folders_and_tags_merge_to_the_lower_gid_in_any_order() {
    for order in [[0usize, 1], [1, 0]] {
        let (hub, a, b) = (node(), node(), node());
        link(&hub, &a);
        link(&hub, &b);
        let (ma, mb) = (done_meeting(&a, "Họp A"), done_meeting(&b, "Họp B"));
        let (fa, fb) = (
            a.store.create_folder("Q4").unwrap().gid,
            b.store.create_folder("Q4").unwrap().gid,
        );
        let (ta, tb) = (
            a.store.create_tag("pricing").unwrap().gid,
            b.store.create_tag("pricing").unwrap().gid,
        );
        a.store
            .set_meeting_folder(std::slice::from_ref(&ma), Some(&fa))
            .unwrap();
        b.store
            .set_meeting_folder(std::slice::from_ref(&mb), Some(&fb))
            .unwrap();
        a.store
            .tag_meetings(std::slice::from_ref(&ma), &ta)
            .unwrap();
        b.store
            .tag_meetings(std::slice::from_ref(&mb), &tb)
            .unwrap();
        let (folder, folder_lost) = if fa < fb { (&fa, &fb) } else { (&fb, &fa) };
        let (tag, tag_lost) = if ta < tb { (&ta, &tb) } else { (&tb, &ta) };

        round(&hub, &[&a, &b], &order);
        round(&hub, &[&a, &b], &order);
        let ms = vec![ma.clone(), mb.clone()];
        for n in [&hub, &a, &b] {
            let o = org(n, &ms);
            assert_eq!(
                o.folders,
                vec![(folder.clone(), "Q4".to_string())],
                "{order:?}"
            );
            assert_eq!(
                o.tags,
                vec![(tag.clone(), "pricing".to_string())],
                "{order:?}"
            );
            for (_, f, t) in &o.meetings {
                assert_eq!(f.as_ref(), Some(folder), "{order:?}");
                assert_eq!(t, &vec![tag.clone()], "{order:?}");
            }
            assert!(n.store.is_tombstoned(folder_lost).unwrap());
            assert!(n.store.is_tombstoned(tag_lost).unwrap());
            assert!(!n.store.is_tombstoned(folder).unwrap());
            assert!(!n.store.is_tombstoned(tag).unwrap());
        }
        // Same everywhere, and another session changes nothing.
        let (h, sa, sb) = (org(&hub, &ms), org(&a, &ms), org(&b, &ms));
        assert_eq!(h, sa);
        assert_eq!(h, sb);
        round(&hub, &[&a, &b], &order);
        assert_eq!(org(&hub, &ms), h);
        assert_eq!(org(&a, &ms), h);
        assert_eq!(org(&b, &ms), h);
    }
}

#[test]
fn sync_late_links_and_folders_naming_a_folded_away_gid_follow_the_survivor() {
    let (hub, a, b) = (node(), node(), node());
    link(&hub, &a);
    link(&hub, &b);
    let m = done_meeting(&a, "Họp");
    let (fa, fb) = (
        a.store.create_folder("Q4").unwrap().gid,
        b.store.create_folder("Q4").unwrap().gid,
    );
    let (ta, tb) = (
        a.store.create_tag("pricing").unwrap().gid,
        b.store.create_tag("pricing").unwrap().gid,
    );
    let (folder, lost_folder) = if fa < fb { (&fa, &fb) } else { (&fb, &fa) };
    let (tag, lost_tag) = if ta < tb { (&ta, &tb) } else { (&tb, &ta) };
    // The hub knows both; the loser is folded away.
    flow(&a, &hub);
    flow(&b, &hub);
    assert!(hub.store.is_tombstoned(lost_folder).unwrap());
    assert!(hub.store.is_tombstoned(lost_tag).unwrap());

    // Later, a device that still uses the loser sends a meeting and a link.
    let (loser_dev, lost_f, lost_t) = if lost_folder == &fa {
        (&a, &fa, &ta)
    } else {
        (&b, &fb, &tb)
    };
    let lost_t = if lost_tag == lost_t { lost_t } else { lost_tag };
    let lost_f = if lost_folder == lost_f {
        lost_f
    } else {
        lost_folder
    };
    let Record::Meeting(mut mr) = rec(&a, "meeting", &m) else {
        unreachable!()
    };
    mr.folder_gid = Some(lost_f.clone());
    mr.version = ver(mr.version.lamport + 1, &a.gid);
    mr.base = None;
    let link_gid = common::gid(14, 3);
    let lk = Record::MeetingTag(MeetingTagRec {
        gid: link_gid.clone(),
        version: ver(5_000, &loser_dev.gid),
        meeting_gid: m.clone(),
        tag_gid: lost_t.clone(),
        ..Default::default()
    });
    push(&hub, &a, vec![Record::Meeting(mr), lk]);
    assert_eq!(
        hub.store.get_meeting(&m).unwrap().folder_gid.as_ref(),
        Some(folder)
    );
    let tags = hub.store.meeting_tags(std::slice::from_ref(&m)).unwrap();
    assert_eq!(
        tags[&m].iter().map(|t| &t.gid).collect::<Vec<_>>(),
        vec![tag]
    );
    assert!(hub.store.is_tombstoned(&link_gid).unwrap());
}

#[test]
fn sync_a_rename_onto_another_folders_name_folds_the_higher_gid() {
    let (hub, a) = (node(), node());
    link(&hub, &a);
    let m = done_meeting(&a, "Họp");
    let f1 = a.store.create_folder("Alpha").unwrap().gid;
    let f2 = a.store.create_folder("Beta").unwrap().gid;
    a.store
        .set_meeting_folder(std::slice::from_ref(&m), Some(&f1))
        .unwrap();
    flow(&a, &hub);
    // The hub's user renames Beta to Alpha's name: a local rename is refused
    // (Duplicate), so the collision comes as a peer's rename.
    let Record::Folder(mut r) = rec(&a, "folder", &f2) else {
        unreachable!()
    };
    r.version = ver(r.version.lamport + 10, &a.gid);
    r.name = Some("alpha".into());
    push(&hub, &a, vec![Record::Folder(r)]);
    let (win, lose) = if f1 < f2 { (&f1, &f2) } else { (&f2, &f1) };
    let folders = hub.store.folders().unwrap();
    assert_eq!(folders.len(), 1);
    assert_eq!(&folders[0].gid, win);
    assert!(hub.store.is_tombstoned(lose).unwrap());
    assert_eq!(
        hub.store.get_meeting(&m).unwrap().folder_gid.as_ref(),
        Some(win)
    );
}

#[test]
fn sync_same_name_people_stay_two_people_and_the_lower_gid_keeps_the_key() {
    for order in [[0usize, 1], [1, 0]] {
        let (hub, a, b) = (node(), node(), node());
        link(&hub, &a);
        link(&hub, &b);
        let pa = a.store.add_person("Linh", 1).unwrap();
        let pb = b.store.add_person("Linh", 2).unwrap();
        round(&hub, &[&a, &b], &order);
        round(&hub, &[&a, &b], &order);
        let low = if pa < pb { &pa } else { &pb };
        for n in [&hub, &a, &b] {
            let conn = n.raw();
            let names: Vec<(String, String)> = conn
                .prepare("SELECT gid, name FROM persons WHERE is_me = 0 ORDER BY gid")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(names.len(), 2, "{order:?}");
            assert!(names.iter().all(|(_, n)| n == "Linh"), "{names:?}");
            let plain: String = conn
                .query_row("SELECT gid FROM persons WHERE name_key = 'linh'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(&plain, low, "{order:?}");
            assert!(!n.store.is_tombstoned(&pa).unwrap());
            assert!(!n.store.is_tombstoned(&pb).unwrap());
        }
    }
}

/// A meeting created on `c`, copied to the hub and to two spokes, with
/// `n` speakers. Returns the nodes, the meeting and the speaker gids.
fn cycle_world(n: usize) -> (Node, Node, Node, Node, String, Vec<String>) {
    let (hub, c, s1, s2) = (node(), node(), node(), node());
    link(&hub, &c);
    link(&hub, &s1);
    link(&hub, &s2);
    let m = done_meeting(&c, "Họp");
    let speakers: Vec<String> = (0..n)
        .map(|i| {
            c.store
                .add_speaker(
                    &m,
                    NewSpeaker {
                        label_idx: i as i64,
                        ..Default::default()
                    },
                )
                .unwrap()
        })
        .collect();
    flow(&c, &hub);
    flow(&hub, &s1);
    flow(&hub, &s2);
    (hub, c, s1, s2, m, speakers)
}

fn merged_of(n: &Node, gid: &str) -> Option<String> {
    n.raw()
        .query_row(
            "SELECT (SELECT x.gid FROM speakers x WHERE x.id = s.merged_into)
             FROM speakers s WHERE s.gid = ?1",
            [gid],
            |r| r.get(0),
        )
        .unwrap()
}

/// Edge `from -> to` at lamport `l`, as the speaker row of `from` carries it.
fn edge(c: &Node, from: &str, to: &str, l: i64) -> Record {
    let Record::Speaker(mut r) = rec(c, "speaker", from) else {
        unreachable!()
    };
    r.version = ver(l, &c.gid);
    r.base = None;
    r.merged_into = Some(to.into());
    Record::Speaker(r)
}

#[test]
fn sync_a_two_speaker_merge_cycle_is_broken_the_same_on_hub_and_spokes() {
    let (hub, c, s1, s2, _m, sp) = cycle_world(2);
    let e1 = edge(&c, &sp[0], &sp[1], 1_000); // S0 -> S1, the lower version
    let e2 = edge(&c, &sp[1], &sp[0], 1_001);
    // Hub: one order; the spokes get the edges from the hub in the others.
    push(&hub, &c, vec![e1.clone(), e2.clone()]);
    for (n, order) in [(&s1, vec![e2.clone(), e1.clone()]), (&s2, vec![e1, e2])] {
        n.store.apply_rows(&hub.gid, &order).unwrap();
    }
    // And the other order on the hub.
    let (hub2, c2, _, _, _, sp2) = cycle_world(2);
    push(
        &hub2,
        &c2,
        vec![
            edge(&c2, &sp2[1], &sp2[0], 1_001),
            edge(&c2, &sp2[0], &sp2[1], 1_000),
        ],
    );
    assert_eq!(merged_of(&hub2, &sp2[0]), None);
    assert_eq!(merged_of(&hub2, &sp2[1]), Some(sp2[0].clone()));
    for n in [&hub, &s1, &s2] {
        assert_eq!(merged_of(n, &sp[0]), None, "the lower edge is cleared");
        assert_eq!(merged_of(n, &sp[1]), Some(sp[0].clone()));
    }
    // A redelivery changes nothing.
    s1.store
        .apply_rows(&hub.gid, &[edge(&c, &sp[0], &sp[1], 1_000)])
        .unwrap();
    assert_eq!(merged_of(&s1, &sp[0]), None);
}

#[test]
fn sync_a_longer_merge_cycle_is_broken_at_its_lowest_version_in_any_order() {
    let (hub, c, s1, s2, _m, sp) = cycle_world(3);
    let e = [
        edge(&c, &sp[0], &sp[1], 1_010),
        edge(&c, &sp[1], &sp[2], 1_011),
        edge(&c, &sp[2], &sp[0], 1_009), // the lowest: the one to go
    ];
    let pick = |ix: [usize; 3]| ix.iter().map(|&i| e[i].clone()).collect::<Vec<_>>();
    push(&hub, &c, pick([0, 1, 2]));
    s1.store.apply_rows(&hub.gid, &pick([2, 1, 0])).unwrap();
    s2.store.apply_rows(&hub.gid, &pick([1, 2, 0])).unwrap();
    for n in [&hub, &s1, &s2] {
        assert_eq!(merged_of(n, &sp[0]), Some(sp[1].clone()));
        assert_eq!(merged_of(n, &sp[1]), Some(sp[2].clone()));
        assert_eq!(merged_of(n, &sp[2]), None);
    }
}

static LOGS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        LOGS.lock().unwrap().push(r.args().to_string());
    }
    fn flush(&self) {}
}

#[test]
fn sync_a_parked_row_that_fails_on_retry_is_logged_by_gid_and_code() {
    static CAP: Capture = Capture;
    let _ = log::set_logger(&CAP);
    log::set_max_level(log::LevelFilter::Info);
    let (hub, s1) = (node(), node());
    link(&hub, &s1);
    let w = world(&s1);
    // A speaker whose name is sealed under a key that is not the meeting's.
    let Record::Speaker(mut sp) = rec(&s1, "speaker", &w.speaker) else {
        unreachable!()
    };
    let wrong = Dek::from_bytes([9u8; 32]);
    sp.display_name_ct = Some(seal(&wrong, "speakers", "display_name_ct", &sp.gid, "Linh"));
    let r = push(&hub, &s1, vec![Record::Speaker(sp.clone())]);
    assert_eq!(outcomes(&r), vec![Parked]);
    assert_eq!(hub.store.pending_count().unwrap(), 1);

    push(&hub, &s1, vec![rec(&s1, "meeting", &w.m)]);
    assert_eq!(hub.store.pending_count().unwrap(), 0, "dropped");
    assert!(hub.store.speakers(&w.m).unwrap().is_empty());
    let logs = LOGS.lock().unwrap().clone();
    let line = logs
        .iter()
        .find(|l| l.contains(&sp.gid))
        .unwrap_or_else(|| panic!("no log line for the dropped row: {logs:?}"));
    assert!(line.contains("bad_record"), "{line}");
    assert!(!line.contains("Linh"), "no content in the log");
}
