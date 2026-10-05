// SPDX-License-Identifier: Apache-2.0
//! Sync sessions between real stores (slice 15-T): a hub and spokes with
//! debug file key stores, joined by an in-memory pipe under Noise. No sockets.

use std::path::Path;
use std::sync::Arc;
use std::thread;

use ghi_store::keys::Protection;
use ghi_store::keys::dev::FileKeyStore;
use ghi_store::store::{NewMeeting, NewNoteBlock, NewSegment, Provenance, Store, TrackKind};
use ghi_sync::SyncStore;
use ghi_sync::clock::SystemClock;
use ghi_sync::identity::{Identity, StaticSecret};
use ghi_sync::mem::mem_pipe;
use ghi_sync::qr::QrPayload;
use ghi_sync::service::{HubNode, Served, pair_over, session_over};
use ghi_sync::session::SessionReport;

struct Node {
    store: Arc<Store>,
    identity: Identity,
    _dir: tempfile::TempDir,
}

fn copy_identity(i: &Identity) -> Identity {
    Identity {
        device_gid: i.device_gid.clone(),
        secret: StaticSecret::from_bytes(*i.secret.as_bytes()),
        public: i.public,
    }
}

fn node() -> Node {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let keys = FileKeyStore::new(dir.path().join("data.devkey"));
    let store = Arc::new(Store::open(&data, Arc::new(keys), Protection::default()).unwrap());
    let mut identity = Identity::generate().unwrap();
    identity.device_gid = store.sync_device_gid().unwrap();
    Node {
        store,
        identity,
        _dir: dir,
    }
}

fn dyn_store(n: &Node) -> Arc<dyn SyncStore> {
    n.store.clone()
}

fn hub_node(n: &Node) -> Arc<HubNode> {
    Arc::new(HubNode::new(
        dyn_store(n),
        copy_identity(&n.identity),
        Arc::new(SystemClock),
        "Mac",
        4455,
    ))
}

/// Pairs `spoke` with the hub over a pipe.
fn pair(hub: &Arc<HubNode>, hub_n: &Node, spoke: &Node) {
    let psk = hub.open_pairing().unwrap();
    let qr = QrPayload {
        v: 1,
        dev: *uuid::Uuid::parse_str(&hub_n.identity.device_gid)
            .unwrap()
            .as_bytes(),
        pk: hub_n.identity.public,
        psk,
        addrs: Vec::new(),
    };
    let (a, b) = mem_pipe();
    let h = hub.clone();
    let server = thread::spawn(move || h.serve(a, None, None));
    let paired = pair_over(
        spoke.store.as_ref(),
        &spoke.identity,
        &qr,
        b,
        "iPhone",
        "ios",
    )
    .unwrap();
    assert_eq!(paired.device_gid, hub_n.identity.device_gid);
    assert!(matches!(
        server.join().unwrap().unwrap(),
        Served::Paired(p) if p.device_gid == spoke.identity.device_gid
    ));
}

/// One spoke session against the hub.
fn sync(hub: &Arc<HubNode>, spoke: &Node) -> (SessionReport, SessionReport) {
    let (a, b) = mem_pipe();
    let h = hub.clone();
    let server = thread::spawn(move || h.serve(a, None, None));
    let mine = session_over(dyn_store(spoke), Arc::new(SystemClock), &spoke.identity, b);
    let theirs = server.join().unwrap();
    // The hub's error is the cause when both fail: show it.
    let (mine, theirs) = match (mine, theirs) {
        (Ok(m), Ok(t)) => (m, t),
        (m, t) => panic!("session failed: spoke {m:?}, hub {t:?}"),
    };
    let Served::Session(theirs) = theirs else {
        panic!("the hub took a session for a pairing");
    };
    (mine, theirs)
}

fn seg(text: &str, t0: i64) -> NewSegment {
    NewSegment {
        t0_ms: t0,
        t1_ms: t0 + 900,
        text: text.into(),
        ..Default::default()
    }
}

/// A finished meeting with a title, three segments and a note.
fn meeting(s: &Store, title: &str) -> String {
    let m = s
        .create_meeting(NewMeeting {
            title: title.into(),
            ..Default::default()
        })
        .unwrap();
    for (i, t) in ["Xin chào mọi người", "Let's start", "Cảm ơn"]
        .iter()
        .enumerate()
    {
        s.add_segment(&m.gid, seg(t, i as i64 * 1000)).unwrap();
    }
    s.add_note_block(
        &m.gid,
        NewNoteBlock {
            kind: "paragraph".into(),
            provenance: Provenance::User,
            body: "Ghi chú: ship it".into(),
            anchors: vec![],
            pinned: false,
        },
    )
    .unwrap();
    s.finish_meeting(&m.gid, 3_000).unwrap();
    m.gid
}

fn texts(s: &Store, gid: &str) -> Vec<String> {
    s.segments(gid)
        .unwrap()
        .into_iter()
        .map(|x| x.text)
        .collect()
}

fn same_meeting(a: &Store, b: &Store, gid: &str) {
    assert_eq!(
        a.get_meeting(gid).unwrap().title,
        b.get_meeting(gid).unwrap().title
    );
    assert_eq!(texts(a, gid), texts(b, gid));
    let notes = |s: &Store| -> Vec<String> {
        s.note_blocks(gid)
            .unwrap()
            .into_iter()
            .map(|n| n.body)
            .collect()
    };
    assert_eq!(notes(a), notes(b));
}

fn idle(r: &SessionReport) -> bool {
    r.rows_pushed == 0 && r.rows_pulled == 0 && r.tombs_pushed == 0 && r.tombs_pulled == 0
}

#[test]
fn a_meeting_goes_to_the_hub_edits_come_back_and_a_third_run_is_idle() {
    let (hub_n, spoke) = (node(), node());
    let hub = hub_node(&hub_n);
    pair(&hub, &hub_n, &spoke);

    let gid = meeting(&spoke.store, "Weekly sync");
    let (mine, theirs) = sync(&hub, &spoke);
    assert!(mine.rows_pushed >= 5, "{mine:?}");
    assert_eq!(mine.rows_pushed, theirs.rows_pushed);
    same_meeting(&spoke.store, &hub_n.store, &gid);
    assert_eq!(hub_n.store.get_meeting(&gid).unwrap().title, "Weekly sync");
    assert_eq!(texts(&hub_n.store, &gid)[1], "Let's start");

    // The hub renames it; the spoke pulls the new title.
    hub_n
        .store
        .set_meeting_title(&gid, "Renamed on the Mac")
        .unwrap();
    let (mine, _) = sync(&hub, &spoke);
    assert!(mine.rows_pulled >= 1, "{mine:?}");
    assert_eq!(
        spoke.store.get_meeting(&gid).unwrap().title,
        "Renamed on the Mac"
    );
    same_meeting(&spoke.store, &hub_n.store, &gid);

    // Nothing changed: nothing is exchanged, in either direction.
    let (mine, theirs) = sync(&hub, &spoke);
    assert!(idle(&mine), "{mine:?}");
    assert!(idle(&theirs), "{theirs:?}");
    // And again (the rows `mark_clean` re-logs must not come round forever).
    let (mine, theirs) = sync(&hub, &spoke);
    assert!(idle(&mine) && idle(&theirs), "{mine:?} {theirs:?}");
}

#[test]
fn a_delete_on_the_spoke_shreds_the_meeting_on_the_hub() {
    let (hub_n, spoke) = (node(), node());
    let hub = hub_node(&hub_n);
    pair(&hub, &hub_n, &spoke);
    let keep = meeting(&spoke.store, "Keep me");
    let gone = meeting(&spoke.store, "Delete me");
    sync(&hub, &spoke);
    assert_eq!(hub_n.store.list_meetings(10, 0).unwrap().len(), 2);

    spoke.store.delete_meeting(&gone).unwrap();
    let (mine, theirs) = sync(&hub, &spoke);
    // The meeting and its three lines and note.
    assert_eq!(mine.tombs_pushed, 5, "{mine:?}");
    assert_eq!(theirs.tombs_pushed, 5);
    assert_eq!(
        mine.tombs_pulled, 0,
        "the spoke's own deletes do not come back"
    );
    // The meeting row and its key are gone: nothing opens it any more.
    assert!(hub_n.store.get_meeting(&gone).is_err());
    assert!(hub_n.store.segments(&gone).is_err());
    assert!(hub_n.store.note_blocks(&gone).is_err());
    assert!(
        hub_n
            .store
            .meeting_dek_for_peer(&spoke.identity.device_gid, &gone)
            .is_err()
    );
    let left = hub_n.store.list_meetings(10, 0).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].gid, keep);
    same_meeting(&spoke.store, &hub_n.store, &keep);

    let (mine, theirs) = sync(&hub, &spoke);
    assert!(idle(&mine) && idle(&theirs), "{mine:?} {theirs:?}");
}

#[test]
fn a_concurrent_title_edit_keeps_the_winner_and_a_conflict_copy() {
    let (hub_n, spoke) = (node(), node());
    let hub = hub_node(&hub_n);
    pair(&hub, &hub_n, &spoke);
    let gid = meeting(&spoke.store, "Planning");
    sync(&hub, &spoke);

    hub_n
        .store
        .set_meeting_title(&gid, "Title from the Mac")
        .unwrap();
    spoke
        .store
        .set_meeting_title(&gid, "Title from the phone")
        .unwrap();
    sync(&hub, &spoke);

    let title = hub_n.store.get_meeting(&gid).unwrap().title;
    assert!(
        title == "Title from the Mac" || title == "Title from the phone",
        "{title}"
    );
    assert_eq!(spoke.store.get_meeting(&gid).unwrap().title, title);
    let loser = if title == "Title from the Mac" {
        "Title from the phone"
    } else {
        "Title from the Mac"
    };
    let copies = hub_n.store.conflict_copies(&gid).unwrap();
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert_eq!(copies[0].text, loser);

    // A redelivery makes no second copy; the spoke ends with the same copy.
    sync(&hub, &spoke);
    assert_eq!(hub_n.store.conflict_copies(&gid).unwrap().len(), 1);
    let on_spoke = spoke.store.conflict_copies(&gid).unwrap();
    assert_eq!(on_spoke.len(), 1);
    assert_eq!(on_spoke[0].text, loser);
    let (mine, theirs) = sync(&hub, &spoke);
    assert!(idle(&mine) && idle(&theirs), "{mine:?} {theirs:?}");
}

#[test]
fn the_hub_relays_between_two_spokes() {
    let (hub_n, a, b) = (node(), node(), node());
    let hub = hub_node(&hub_n);
    pair(&hub, &hub_n, &a);
    pair(&hub, &hub_n, &b);

    let gid = meeting(&a.store, "From phone A");
    sync(&hub, &a);
    let (mine, _) = sync(&hub, &b);
    assert!(mine.rows_pulled >= 5, "{mine:?}");
    same_meeting(&a.store, &b.store, &gid);
    // B edits a note-free field; A gets it through the hub.
    b.store.set_meeting_title(&gid, "Edited on B").unwrap();
    sync(&hub, &b);
    sync(&hub, &a);
    assert_eq!(a.store.get_meeting(&gid).unwrap().title, "Edited on B");
    assert_eq!(hub_n.store.get_meeting(&gid).unwrap().title, "Edited on B");

    // A delete on B reaches A the same way, and B's copy stays dead.
    b.store.delete_meeting(&gid).unwrap();
    sync(&hub, &b);
    sync(&hub, &a);
    assert!(a.store.get_meeting(&gid).is_err());
    assert!(hub_n.store.get_meeting(&gid).is_err());
    for n in [&a, &b] {
        let (mine, theirs) = sync(&hub, n);
        assert!(idle(&mine) && idle(&theirs), "{mine:?} {theirs:?}");
    }
}

fn record_track(s: &Store, gid: &str, kind: TrackKind, pages: usize) {
    let mut w = s.open_track(gid, kind).unwrap();
    for i in 0..pages {
        w.append(&vec![(i % 251) as u8; 700 + i]).unwrap();
    }
    s.finish_track(gid, kind, w).unwrap();
}

fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap()
}

#[test]
fn a_finished_phone_track_arrives_byte_identical_and_is_not_offered_again() {
    let (hub_n, spoke) = (node(), node());
    let hub = hub_node(&hub_n);
    pair(&hub, &hub_n, &spoke);

    let m = spoke
        .store
        .create_meeting(NewMeeting {
            title: "With audio".into(),
            ..Default::default()
        })
        .unwrap();
    // 300 pages: more than one TrackPages message.
    record_track(&spoke.store, &m.gid, TrackKind::Mic, 300);
    spoke.store.add_segment(&m.gid, seg("hello", 0)).unwrap();
    spoke.store.finish_meeting(&m.gid, 6_000).unwrap();

    let (mine, theirs) = sync(&hub, &spoke);
    assert_eq!(mine.tracks_sent, 1, "{mine:?}");
    assert_eq!(theirs.tracks_received.len(), 1);
    let src = spoke.store.bundle_path(&m.gid, TrackKind::Mic).unwrap();
    let dst = hub_n.store.bundle_path(&m.gid, TrackKind::Mic).unwrap();
    assert_eq!(read(&src), read(&dst));
    assert!(!ghi_store::bundle::part_path(&dst).exists());
    let pages = hub_n
        .store
        .open_bundle(&m.gid, TrackKind::Mic)
        .unwrap()
        .page_count();
    assert_eq!(pages, 300);
    assert_eq!(
        hub_n.store.tracks(&m.gid).unwrap(),
        vec![(TrackKind::Mic, 300)]
    );

    let (mine, theirs) = sync(&hub, &spoke);
    assert_eq!(mine.tracks_sent, 0, "{mine:?}");
    assert!(theirs.tracks_received.is_empty());
    assert!(idle(&mine) && idle(&theirs), "{mine:?} {theirs:?}");
}
