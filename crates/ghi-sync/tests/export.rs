// SPDX-License-Identifier: Apache-2.0
//! "Export for another device" between real stores (slice 15-M): export, import
//! into a fresh store, then a LAN sync (an in-memory pipe under Noise) that
//! must find nothing to fix.

use std::path::Path;
use std::sync::Arc;
use std::thread;

use ghi_store::export::KdfParams;
use ghi_store::keys::Protection;
use ghi_store::keys::dev::FileKeyStore;
use ghi_store::store::{NewMeeting, NewNoteBlock, NewSegment, Provenance, Store, TrackKind};
use ghi_sync::SyncStore;
use ghi_sync::clock::SystemClock;
use ghi_sync::export::{ExportReport, export_for_device_with, import_from_device};
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

fn record_track(s: &Store, gid: &str, kind: TrackKind, pages: usize) {
    let mut w = s.open_track(gid, kind).unwrap();
    for i in 0..pages {
        w.append(&vec![(i % 251) as u8; 700 + i]).unwrap();
    }
    s.finish_track(gid, kind, w).unwrap();
}

/// A meeting with a title, three segments, a note and 150 audio pages (more
/// than one batch of raw records).
fn meeting_with_audio(s: &Store, title: &str) -> String {
    let m = s
        .create_meeting(NewMeeting {
            title: title.into(),
            ..Default::default()
        })
        .unwrap();
    record_track(s, &m.gid, TrackKind::Mic, 150);
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

fn notes(s: &Store, gid: &str) -> Vec<String> {
    s.note_blocks(gid)
        .unwrap()
        .into_iter()
        .map(|n| n.body)
        .collect()
}

fn same_audio(a: &Store, b: &Store, gid: &str) {
    let path = |s: &Store| s.bundle_path(gid, TrackKind::Mic).unwrap();
    assert_eq!(
        std::fs::read(path(a)).unwrap(),
        std::fs::read(path(b)).unwrap()
    );
    assert_eq!(a.tracks(gid).unwrap(), b.tracks(gid).unwrap());
}

const PASS: &str = "correct horse battery";

/// Cheap Argon2 for tests (the import reads the cost from the file).
fn fast() -> KdfParams {
    KdfParams {
        m_kib: 64,
        t: 1,
        p: 1,
    }
}

fn export(n: &Node, only: Option<&[String]>, out: &Path) -> ExportReport {
    export_for_device_with(&n.store, only, PASS, out, &fast()).unwrap()
}

/// Everything a store holds that an import could change.
fn fingerprint(s: &Store) -> (Vec<String>, i64, Vec<String>) {
    let meetings = s
        .list_meetings(1000, 0)
        .unwrap()
        .into_iter()
        .map(|m| m.gid)
        .collect();
    let log = s.changes_since(0, 256).unwrap().upto_seq;
    let leftovers = std::fs::read_dir(s.dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".ghi-import"))
        .collect();
    (meetings, log, leftovers)
}

#[test]
fn an_export_imports_into_a_fresh_store_and_a_lan_sync_finds_nothing_to_fix() {
    let (a, b) = (node(), node());
    let gid = meeting_with_audio(&a.store, "Weekly sync");
    let out = a._dir.path().join("for-b.ghix");

    let wrote = export(&a, None, &out);
    assert_eq!(wrote.meetings, 1);
    assert_eq!(wrote.tracks, 1);
    assert!(wrote.audio_bytes > 150 * 700);

    let took = import_from_device(&b.store, &out, PASS).unwrap();
    assert_eq!((took.meetings, took.refused, took.tracks), (1, 0, 1));
    assert_eq!(took.audio_bytes, wrote.audio_bytes);
    same_meeting(&a.store, &b.store, &gid);
    same_audio(&a.store, &b.store, &gid);
    assert!(
        !ghi_store::bundle::part_path(&b.store.bundle_path(&gid, TrackKind::Mic).unwrap()).exists()
    );
    assert!(
        fingerprint(&b.store).2.is_empty(),
        "scratch directory left behind"
    );

    // Now the devices can reach each other after all: nothing conflicts,
    // nothing is duplicated, and the next run is idle.
    let hub = hub_node(&a);
    pair(&hub, &a, &b);
    sync(&hub, &b);
    let (mine, theirs) = sync(&hub, &b);
    assert!(idle(&mine) && idle(&theirs), "{mine:?} {theirs:?}");
    for n in [&a, &b] {
        assert_eq!(n.store.list_meetings(100, 0).unwrap().len(), 1);
        assert!(n.store.conflict_copies(&gid).unwrap().is_empty());
        assert_eq!(texts(&n.store, &gid).len(), 3);
        assert_eq!(notes(&n.store, &gid).len(), 1);
    }
    same_meeting(&a.store, &b.store, &gid);
    same_audio(&a.store, &b.store, &gid);

    // An edit on either side still flows afterwards.
    b.store.set_meeting_title(&gid, "Renamed on B").unwrap();
    sync(&hub, &b);
    assert_eq!(a.store.get_meeting(&gid).unwrap().title, "Renamed on B");
}

#[test]
fn only_the_chosen_meetings_are_exported() {
    let (a, b) = (node(), node());
    let one = meeting_with_audio(&a.store, "One");
    let two = meeting_with_audio(&a.store, "Two");
    let out = a._dir.path().join("one.ghix");
    let wrote = export(&a, Some(std::slice::from_ref(&one)), &out);
    assert_eq!((wrote.meetings, wrote.tracks), (1, 1));

    import_from_device(&b.store, &out, PASS).unwrap();
    assert_eq!(b.store.list_meetings(100, 0).unwrap().len(), 1);
    same_meeting(&a.store, &b.store, &one);
    assert!(b.store.get_meeting(&two).is_err());
}

#[test]
fn a_gid_tombstoned_on_the_importing_device_is_refused() {
    let (a, b) = (node(), node());
    let keep = meeting_with_audio(&a.store, "Keep");
    let gone = meeting_with_audio(&a.store, "Deleted on B");
    let out = a._dir.path().join("f.ghix");
    export(&a, None, &out);

    // B had the meeting, deleted it, and must not get it back.
    import_from_device(&b.store, &out, PASS).unwrap();
    b.store.delete_meeting(&gone).unwrap();
    let again = import_from_device(&b.store, &out, PASS).unwrap();
    assert_eq!((again.meetings, again.refused), (1, 1));
    assert!(b.store.get_meeting(&gone).is_err());
    assert!(b.store.is_tombstoned(&gone).unwrap());
    assert!(!b.store.bundle_path(&gone, TrackKind::Mic).unwrap().exists());
    same_meeting(&a.store, &b.store, &keep);

    // And on a store that never had it: its tombstone also reaches A's file
    // as a deletion only when A deleted it.
    let c = node();
    a.store.delete_meeting(&gone).unwrap();
    let out2 = a._dir.path().join("g.ghix");
    export(&a, None, &out2);
    import_from_device(&c.store, &out2, PASS).unwrap();
    assert!(c.store.get_meeting(&gone).is_err());
    assert_eq!(c.store.list_meetings(100, 0).unwrap().len(), 1);
}

#[test]
fn a_wrong_passphrase_or_a_damaged_file_changes_nothing() {
    let (a, b) = (node(), node());
    meeting_with_audio(&a.store, "Secret");
    let out = a._dir.path().join("f.ghix");
    export(&a, None, &out);
    let before = fingerprint(&b.store);

    let wrong = import_from_device(&b.store, &out, "not the passphrase");
    assert!(
        matches!(
            wrong,
            Err(ghi_sync::SyncError::Store(ghi_store::StoreError::Decrypt))
        ),
        "{wrong:?}"
    );
    assert_eq!(fingerprint(&b.store), before);
    assert!(b.store.devices().unwrap().is_empty());

    // A flipped byte near the end (inside the audio): rejected before any apply.
    let mut bytes = std::fs::read(&out).unwrap();
    let at = bytes.len() - 40;
    bytes[at] ^= 0x55;
    let bad = b._dir.path().join("bad.ghix");
    std::fs::write(&bad, bytes).unwrap();
    assert!(import_from_device(&b.store, &bad, PASS).is_err());
    assert_eq!(fingerprint(&b.store), before);

    // Not an export at all.
    let junk = b._dir.path().join("junk.ghix");
    std::fs::write(&junk, b"hello").unwrap();
    assert!(import_from_device(&b.store, &junk, PASS).is_err());
    assert_eq!(fingerprint(&b.store), before);
}

#[test]
fn importing_the_same_file_twice_is_a_no_op() {
    let (a, b) = (node(), node());
    let gid = meeting_with_audio(&a.store, "Once");
    let out = a._dir.path().join("f.ghix");
    export(&a, None, &out);

    import_from_device(&b.store, &out, PASS).unwrap();
    let once = fingerprint(&b.store);
    let audio = std::fs::read(b.store.bundle_path(&gid, TrackKind::Mic).unwrap()).unwrap();

    let second = import_from_device(&b.store, &out, PASS).unwrap();
    assert_eq!((second.meetings, second.refused, second.tracks), (1, 0, 1));
    assert_eq!(fingerprint(&b.store), once);
    assert_eq!(
        std::fs::read(b.store.bundle_path(&gid, TrackKind::Mic).unwrap()).unwrap(),
        audio
    );
    assert_eq!(texts(&b.store, &gid).len(), 3);
    assert_eq!(notes(&b.store, &gid).len(), 1);
}

#[test]
fn a_short_passphrase_and_an_empty_selection_are_refused() {
    let a = node();
    let out = a._dir.path().join("f.ghix");
    // Nothing to export yet.
    assert!(export_for_device_with(&a.store, None, PASS, &out, &fast()).is_err());
    let gid = meeting_with_audio(&a.store, "x");
    assert!(export_for_device_with(&a.store, None, "short", &out, &fast()).is_err());
    assert!(!out.exists());
    assert!(
        export_for_device_with(&a.store, Some(&["nope".to_string()]), PASS, &out, &fast()).is_err()
    );
    assert!(export_for_device_with(&a.store, Some(&[gid]), PASS, &out, &fast()).is_ok());
}
