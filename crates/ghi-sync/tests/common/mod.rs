// SPDX-License-Identifier: Apache-2.0
//! Shared harness of the 15-L verification tests: real stores in temp dirs
//! (debug file key stores), paired over an in-memory pipe under Noise, with a
//! wrapper stream that can cut a connection after N frames. No sockets.
#![allow(dead_code)]

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use ghi_store::keys::Protection;
use ghi_store::keys::dev::FileKeyStore;
use ghi_store::store::{NewMeeting, NewNoteBlock, NewSegment, Provenance, Store, TrackKind};
use ghi_sync::SyncStore;
use ghi_sync::clock::{Clock, SystemClock};
use ghi_sync::identity::{Identity, StaticSecret};
use ghi_sync::mem::mem_pipe;
use ghi_sync::qr::QrPayload;
use ghi_sync::service::{HubNode, Served, pair_over, session_over};
use ghi_sync::session::SessionReport;
use ghi_sync::transport::ByteStream;
use ghi_sync::{Result as SyncResult, SyncError};

/// One device: a store in its own temp dir and a sync identity.
pub struct Node {
    store: Option<Arc<Store>>,
    pub identity: Identity,
    pub dir: tempfile::TempDir,
}

pub fn copy_identity(i: &Identity) -> Identity {
    Identity {
        device_gid: i.device_gid.clone(),
        secret: StaticSecret::from_bytes(*i.secret.as_bytes()),
        public: i.public,
    }
}

fn open_store(dir: &Path) -> Arc<Store> {
    let keys = FileKeyStore::new(dir.join("data.devkey"));
    Arc::new(Store::open(&dir.join("data"), Arc::new(keys), Protection::default()).unwrap())
}

pub fn node() -> Node {
    let dir = tempfile::tempdir().unwrap();
    let store = open_store(dir.path());
    let mut identity = Identity::generate().unwrap();
    identity.device_gid = store.sync_device_gid().unwrap();
    Node {
        store: Some(store),
        identity,
        dir,
    }
}

impl Node {
    pub fn store(&self) -> &Arc<Store> {
        self.store.as_ref().expect("the store is open")
    }

    pub fn gid(&self) -> &str {
        &self.identity.device_gid
    }

    pub fn data_dir(&self) -> std::path::PathBuf {
        self.dir.path().join("data")
    }

    /// A process restart: closes the store (every other holder must be gone)
    /// and opens the same directory again.
    pub fn reopen(&mut self) {
        drop(self.store.take());
        self.store = Some(open_store(self.dir.path()));
    }

    pub fn dyn_store(&self) -> Arc<dyn SyncStore> {
        self.store().clone()
    }

    pub fn hub(&self) -> Arc<HubNode> {
        self.hub_with_clock(Arc::new(SystemClock))
    }

    pub fn hub_with_clock(&self, clock: Arc<dyn Clock>) -> Arc<HubNode> {
        Arc::new(HubNode::new(
            self.dyn_store(),
            copy_identity(&self.identity),
            clock,
            "Mac",
            4455,
        ))
    }
}

/// The QR payload a hub shows while its pairing window is open.
pub fn qr_of(hub: &HubNode, hub_n: &Node) -> QrPayload {
    let psk = hub.open_pairing().unwrap();
    QrPayload {
        v: 1,
        dev: *uuid::Uuid::parse_str(hub_n.gid()).unwrap().as_bytes(),
        pk: hub_n.identity.public,
        psk,
        addrs: Vec::new(),
    }
}

/// Pairs `spoke` with the hub over a pipe.
pub fn pair(hub: &Arc<HubNode>, hub_n: &Node, spoke: &Node) {
    let qr = qr_of(hub, hub_n);
    let (a, b) = mem_pipe();
    let h = hub.clone();
    let server = thread::spawn(move || h.serve(a, None, None));
    let paired = pair_over(
        spoke.store().as_ref(),
        &spoke.identity,
        &qr,
        b,
        "iPhone",
        "ios",
    )
    .unwrap();
    assert_eq!(paired.device_gid, hub_n.gid());
    assert!(matches!(
        server.join().unwrap().unwrap(),
        Served::Paired(p) if p.device_gid == spoke.gid()
    ));
}

/// A byte stream that dies after `limit` writes (frames: every Noise frame is
/// one `write_all`, and so is each handshake message). The peer sees the pipe
/// close. `None`: never.
pub struct Flaky<S: ByteStream> {
    inner: Option<S>,
    writes_left: Option<usize>,
    count: Arc<AtomicUsize>,
}

impl<S: ByteStream> Flaky<S> {
    pub fn new(inner: S, limit: Option<usize>) -> Self {
        Self {
            inner: Some(inner),
            writes_left: limit,
            count: Arc::default(),
        }
    }

    /// Frames written so far (shared: read it after the stream is gone).
    pub fn counter(&self) -> Arc<AtomicUsize> {
        self.count.clone()
    }
}

impl<S: ByteStream> Read for Flaky<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.inner.as_mut() {
            Some(s) => s.read(buf),
            None => Err(io::ErrorKind::ConnectionReset.into()),
        }
    }
}

impl<S: ByteStream> Write for Flaky<S> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if let Some(n) = self.writes_left.as_mut() {
            if *n == 0 {
                // The cable is pulled: the peer's reads end.
                self.inner = None;
            } else {
                *n -= 1;
            }
        }
        if self.inner.is_some() {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
        match self.inner.as_mut() {
            Some(s) => s.write(data),
            None => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.inner.as_mut() {
            Some(s) => s.flush(),
            None => Ok(()),
        }
    }
}

impl<S: ByteStream> ByteStream for Flaky<S> {
    fn set_io_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        match self.inner.as_mut() {
            Some(s) => s.set_io_timeout(timeout),
            None => Ok(()),
        }
    }
}

/// What one attempted session came to, from both ends.
pub struct Ran {
    pub spoke: SyncResult<SessionReport>,
    pub hub: SyncResult<Served>,
    /// Frames each end got onto the wire (handshake messages included).
    pub spoke_frames: usize,
    pub hub_frames: usize,
}

impl Ran {
    pub fn ok(self) -> (SessionReport, SessionReport) {
        match (self.spoke, self.hub) {
            (Ok(m), Ok(Served::Session(t))) => (m, t),
            (m, t) => panic!("session failed: spoke {m:?}, hub {t:?}"),
        }
    }
}

/// One spoke session, optionally cut after `cut` writes by the spoke (or, with
/// `cut_hub`, by the hub). Never panics; both ends' results are returned.
pub fn try_sync(hub: &Arc<HubNode>, spoke: &Node, cut: Option<usize>, cut_hub: bool) -> Ran {
    let (a, b) = mem_pipe();
    let h = hub.clone();
    let hub_limit = if cut_hub { cut } else { None };
    let spoke_limit = if cut_hub { None } else { cut };
    let hub_stream = Flaky::new(a, hub_limit);
    let hub_count = hub_stream.counter();
    let server = thread::spawn(move || h.serve(hub_stream, None, None));
    let spoke_stream = Flaky::new(b, spoke_limit);
    let spoke_count = spoke_stream.counter();
    let mine = session_over(
        spoke.dyn_store(),
        Arc::new(SystemClock),
        &spoke.identity,
        spoke_stream,
    );
    let theirs = server.join().unwrap();
    Ran {
        spoke: mine,
        hub: theirs,
        spoke_frames: spoke_count.load(Ordering::SeqCst),
        hub_frames: hub_count.load(Ordering::SeqCst),
    }
}

/// One complete spoke session with the spoke's own clock (the hub's is the
/// one its node was built with).
pub fn sync_with_clock(
    hub: &Arc<HubNode>,
    spoke: &Node,
    clock: Arc<dyn Clock>,
) -> (SessionReport, SessionReport) {
    let (a, b) = mem_pipe();
    let h = hub.clone();
    let server = thread::spawn(move || h.serve(a, None, None));
    let mine = session_over(spoke.dyn_store(), clock, &spoke.identity, b);
    let theirs = server.join().unwrap();
    Ran {
        spoke: mine,
        hub: theirs,
        spoke_frames: 0,
        hub_frames: 0,
    }
    .ok()
}

/// A full session that must succeed.
pub fn sync(hub: &Arc<HubNode>, spoke: &Node) -> (SessionReport, SessionReport) {
    try_sync(hub, spoke, None, false).ok()
}

pub fn idle(r: &SessionReport) -> bool {
    r.rows_pushed == 0
        && r.rows_pulled == 0
        && r.tombs_pushed == 0
        && r.tombs_pulled == 0
        && r.tracks_sent == 0
        && r.tracks_received.is_empty()
}

pub fn seg(text: &str, t0: i64) -> NewSegment {
    NewSegment {
        t0_ms: t0,
        t1_ms: t0 + 900,
        text: text.into(),
        ..Default::default()
    }
}

pub fn note(body: &str, provenance: Provenance) -> NewNoteBlock {
    NewNoteBlock {
        kind: "paragraph".into(),
        provenance,
        body: body.into(),
        anchors: vec![],
        pinned: false,
    }
}

/// A finished meeting with a title, three segments and a user note.
pub fn meeting(s: &Store, title: &str) -> String {
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
    s.add_note_block(&m.gid, note("Ghi chú: ship it", Provenance::User))
        .unwrap();
    s.finish_meeting(&m.gid, 3_000).unwrap();
    m.gid
}

/// Records a finished track of `pages` pages (more than 255 span messages).
pub fn record_track(s: &Store, gid: &str, kind: TrackKind, pages: usize) {
    record_track_sized(s, gid, kind, pages, 700);
}

/// A track whose pages are about `size` bytes each and all different.
pub fn record_track_sized(s: &Store, gid: &str, kind: TrackKind, pages: usize, size: usize) {
    let mut w = s.open_track(gid, kind).unwrap();
    for i in 0..pages {
        let fill = (i * 7 % 251) as u8;
        let mut page = vec![fill; size + i % 64];
        page[0] = (i % 256) as u8;
        w.append(&page).unwrap();
    }
    s.finish_track(gid, kind, w).unwrap();
}

pub fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap()
}

/// A meeting with one track of `pages` pages, finished.
pub fn meeting_with_audio(s: &Store, title: &str, pages: usize) -> String {
    let m = s
        .create_meeting(NewMeeting {
            title: title.into(),
            ..Default::default()
        })
        .unwrap();
    record_track(s, &m.gid, TrackKind::Mic, pages);
    s.add_segment(&m.gid, seg("hello there", 0)).unwrap();
    s.finish_meeting(&m.gid, 6_000).unwrap();
    m.gid
}

pub fn texts(s: &Store, gid: &str) -> Vec<String> {
    s.segments(gid)
        .unwrap()
        .into_iter()
        .map(|x| x.text)
        .collect()
}

pub fn is_closed(e: &SyncError) -> bool {
    matches!(
        e,
        SyncError::Closed | SyncError::Io(_) | SyncError::Timeout | SyncError::Noise(_)
    )
}
