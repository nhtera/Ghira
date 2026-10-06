// SPDX-License-Identifier: Apache-2.0
//! The sync service against a real [`Core`] (a hub with its job runner, fake
//! speech engines and a scripted notes model) and a phone's own store, joined
//! by in-memory pipes under Noise. No sockets, except where a test needs the
//! listener itself (it then uses the machine's private address, and says so
//! if there is none).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ghi_audio::Track;
use ghi_core::capture::{ReplayTrack, replay};
use ghi_core::engines::{FakeEngines, Script, SpeechEngines};
use ghi_core::events::bus;
use ghi_core::final_pass::FinalPassJob;
use ghi_core::jobs::{JobHandler, always_ready};
use ghi_core::live::Mode;
use ghi_core::notes_job::{NOTES_FINAL_JOB, NotesJob};
use ghi_core::session::{FINAL_PASS_JOB, Session, SessionConfig};
use ghi_llm::{Completion, EngineInfo, Llm, Request};
use ghi_speech::SpeakerSegment;
use ghi_store::keys::Protection;
use ghi_store::keys::dev::FileKeyStore;
use ghi_store::store::{NewMeeting, NewSegment, Store};
use ghi_store::sync::records::Record;
use ghi_sync::SyncStore;
use ghi_sync::clock::SystemClock;
use ghi_sync::identity::Identity;
use ghi_sync::lease::Grantor;
use ghi_sync::mem::mem_pipe;
use ghi_sync::service::{Served, pair_over, session_over};
use ghi_sync::session::SessionReport;

use super::*;
use crate::core::{Core, CoreHooks};

/// The listener's socket count is process-wide: tests that bind take turns.
static NET: Mutex<()> = Mutex::new(());

struct OneLiner;

impl Llm for OneLiner {
    fn engine(&self) -> EngineInfo {
        EngineInfo {
            name: "scripted".into(),
            version: "1".into(),
        }
    }
    fn context_tokens(&self) -> u32 {
        16_384
    }
    fn complete(&mut self, _req: &Request) -> ghi_llm::Result<Completion> {
        Ok(Completion {
            text: r#"{"tldr":[{"text":"Chốt lịch beta","cite":[0]}],"decisions":[],"action_items":[],"open_questions":[],"key_quotes":[],"topics":[]}"#.into(),
            tokens_in: 10,
            tokens_out: 5,
            truncated: false,
        })
    }
}

pub(super) fn script() -> Script {
    Script {
        utterances: vec![(1.0, 2.0, "xin chào mọi người".into())],
        turns: vec![SpeakerSegment {
            start: 0.5,
            end: 2.5,
            speaker: 1,
        }],
    }
}

pub(super) fn handlers(_: &Arc<Store>, _: &std::path::Path) -> Vec<Arc<dyn JobHandler>> {
    let engines: Arc<dyn SpeechEngines> = FakeEngines::new(script());
    let llm: ghi_core::notes_job::LlmFactory =
        Arc::new(|_| Ok(Box::new(OneLiner) as Box<dyn Llm + Send>));
    vec![
        Arc::new(FinalPassJob {
            engines: Arc::new(move || Ok(engines.clone())),
            chunk_s: 600.0,
            ready: always_ready(),
            voice: None,
        }),
        Arc::new(NotesJob {
            kind: NOTES_FINAL_JOB,
            version: 2,
            template: ghi_llm::template::builtin("general").unwrap(),
            llm,
            ready: always_ready(),
        }),
    ]
}

pub(super) struct Hub {
    pub(super) core: Arc<Core>,
    pub(super) svc: Arc<SyncService>,
    pub(super) events: Arc<Mutex<Vec<SyncEvent>>>,
    pub(super) settings_calls: Arc<AtomicUsize>,
    _tmp: tempfile::TempDir,
}

impl Hub {
    pub(super) fn store(&self) -> Arc<Store> {
        self.core.store().unwrap()
    }

    pub(super) fn events(&self) -> Vec<SyncEvent> {
        lock(&self.events).clone()
    }

    pub(super) fn gid(&self) -> String {
        self.store().sync_device_gid().unwrap()
    }
}

pub(super) fn hub_with(addrs: Vec<IpAddr>, reachable_within: Duration) -> Hub {
    hub_with_handlers(addrs, reachable_within, Arc::new(handlers))
}

pub(super) type Handlers =
    Arc<dyn Fn(&Arc<Store>, &std::path::Path) -> Vec<Arc<dyn JobHandler>> + Send + Sync>;

pub(super) fn hub_with_handlers(
    addrs: Vec<IpAddr>,
    reachable_within: Duration,
    handlers: Handlers,
) -> Hub {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _rx) = Core::for_test_with(
        tmp.path().join("data"),
        CoreHooks {
            handlers: Some(handlers),
            recover_kinds: Some(Vec::new()),
            ..Default::default()
        },
    );
    let events: Arc<Mutex<Vec<SyncEvent>>> = Arc::default();
    let settings_calls = Arc::new(AtomicUsize::new(0));
    let (e2, s2) = (events.clone(), settings_calls.clone());
    let mut cfg = SyncConfig::hub(
        "Mac",
        move |e| lock(&e2).push(e),
        move || {
            s2.fetch_add(1, Ordering::SeqCst);
        },
    );
    cfg.addrs = Arc::new(move || addrs.clone());
    cfg.advertise = false;
    cfg.reachable_within = reachable_within;
    let svc = SyncService::new(core.clone(), cfg);
    Hub {
        core,
        svc,
        events,
        settings_calls,
        _tmp: tmp,
    }
}

pub(super) fn hub() -> Hub {
    hub_with(Vec::new(), REACHABLE_WITHIN)
}

struct Spoke {
    store: Arc<Store>,
    identity: Identity,
    _dir: tempfile::TempDir,
}

fn spoke() -> Spoke {
    let dir = tempfile::tempdir().unwrap();
    let keys = FileKeyStore::new(dir.path().join("data.devkey"));
    let store = Arc::new(
        Store::open(
            &dir.path().join("data"),
            Arc::new(keys),
            Protection::default(),
        )
        .unwrap(),
    );
    let mut identity = Identity::generate().unwrap();
    identity.device_gid = store.sync_device_gid().unwrap();
    Spoke {
        store,
        identity,
        _dir: dir,
    }
}

/// Opens the pairing window (as `pair_open` does, without a listener's
/// addresses) and pairs `spoke` over a pipe.
fn pair(hub: &Hub, spoke: &Spoke) {
    hub.svc.set_enabled(true).unwrap();
    lock(&hub.svc.state).pairing_until = Some(Instant::now() + PAIR_TTL);
    hub.svc.reconcile();
    let node = hub
        .svc
        .node()
        .expect("the pairing window brings the node up");
    let text = node.open_pairing_code(&[]).unwrap();
    let qr = ghi_sync::qr::parse(&text).unwrap();
    let (a, b) = mem_pipe();
    let (svc, n) = (hub.svc.clone(), node.clone());
    let server = thread::spawn(move || svc.serve_connection(&n, a, None, None));
    pair_over(
        spoke.store.as_ref(),
        &spoke.identity,
        &qr,
        b,
        "iPhone",
        "ios",
    )
    .unwrap();
    assert!(matches!(server.join().unwrap().unwrap(), Served::Paired(_)));
}

/// One spoke session; the hub's report is what `serve_connection` returned.
fn sync(hub: &Hub, spoke: &Spoke) -> (SessionReport, SessionReport) {
    let node = hub.svc.node().expect("the hub is serving");
    let (a, b) = mem_pipe();
    let svc = hub.svc.clone();
    let server = thread::spawn(move || svc.serve_connection(&node, a, None, None));
    let mine = session_over(
        spoke.store.clone() as Arc<dyn SyncStore>,
        Arc::new(SystemClock),
        &spoke.identity,
        b,
    );
    let theirs = server.join().unwrap();
    match (mine, theirs) {
        (Ok(m), Ok(Served::Session(t))) => (m, t),
        (m, t) => panic!("session failed: spoke {m:?}, hub {t:?}"),
    }
}

/// Like [`sync`], with both outcomes as they are.
fn sync_raw(
    hub: &Hub,
    spoke: &Spoke,
) -> (ghi_sync::Result<SessionReport>, ghi_sync::Result<Served>) {
    let node = hub.svc.node().expect("the hub is serving");
    let (a, b) = mem_pipe();
    let svc = hub.svc.clone();
    let server = thread::spawn(move || svc.serve_connection(&node, a, None, None));
    let mine = session_over(
        spoke.store.clone() as Arc<dyn SyncStore>,
        Arc::new(SystemClock),
        &spoke.identity,
        b,
    );
    (mine, server.join().unwrap())
}

pub(super) fn wait_for(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(secs);
    while !ok() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(50));
    }
}

/// A recorded meeting on the phone: live lines and one audio track.
fn record_on_spoke(spoke: &Spoke) -> String {
    record_meeting(&spoke.store)
}

/// A recorded meeting in `store`: live lines and one audio track.
pub(super) fn record_meeting(store: &Arc<Store>) -> String {
    let (tx, rx) = bus();
    let engines: Arc<dyn SpeechEngines> = FakeEngines::new(script());
    let capture = replay(
        vec![ReplayTrack {
            track: Track::Mic,
            samples: (0..48_000 * 4)
                .map(|i| (i as f32 * 0.03).sin() * 0.2)
                .collect(),
            sample_rate: 48_000,
        }],
        None,
    )
    .unwrap();
    let s = Session::start(
        store.clone(),
        Some(engines),
        capture,
        SessionConfig {
            sensitive: false,
            mode: Mode::Room,
            language: None,
            title: "From the phone".into(),
            queue_jobs: false,
            lossless: true,
            echo_cancellation: true,
        },
        tx,
        None,
    )
    .unwrap();
    let meeting = s.meeting().to_string();
    let t = Instant::now();
    while store.segments(&meeting).unwrap().is_empty() {
        assert!(t.elapsed() < Duration::from_secs(20));
        let _ = rx.recv_timeout(Duration::from_millis(50));
    }
    s.stop().unwrap();
    meeting
}

pub(super) fn texts(s: &Store, meeting: &str) -> Vec<String> {
    s.segments(meeting)
        .unwrap()
        .into_iter()
        .map(|x| x.text)
        .collect()
}

#[test]
fn a_phone_meeting_syncs_the_hub_runs_the_leased_pass_and_the_phone_pulls_v2() {
    let (hub, spoke) = (hub(), spoke());
    pair(&hub, &spoke);
    assert!(
        hub.events()
            .iter()
            .any(|e| matches!(e, SyncEvent::Paired { device } if device.name == "iPhone")),
        "{:?}",
        hub.events()
    );
    assert_eq!(hub.svc.devices().unwrap().len(), 1);

    let meeting = record_on_spoke(&spoke);
    let live_version = spoke
        .store
        .get_meeting(&meeting)
        .unwrap()
        .transcript_version;
    let (mine, theirs) = sync(&hub, &spoke);
    assert_eq!(mine.tracks_sent, 1, "{mine:?}");
    assert_eq!(theirs.tracks_received.len(), 1);
    sync(&hub, &spoke);

    // The phone offers the final pass and the notes (audio and key are acked).
    let hub_gid = hub.gid();
    Grantor
        .offer(
            spoke.store.as_ref(),
            &hub_gid,
            &meeting,
            &[FINAL_PASS_JOB.to_string(), NOTES_FINAL_JOB.to_string()],
            12 * 3_600_000,
        )
        .unwrap();
    let (_, theirs) = sync(&hub, &spoke);
    assert_eq!(theirs.new_leases.len(), 1);
    let lease_id = theirs.new_leases[0].clone();

    // The hub queued the pass under that lease (and its epoch).
    let store = hub.store();
    let jobs = store.jobs_for_meeting(&meeting).unwrap();
    let pass = jobs
        .iter()
        .find(|j| j.kind == FINAL_PASS_JOB)
        .expect("a leased final pass");
    assert_eq!(pass.payload["lease"], lease_id.as_str());
    assert_eq!(pass.payload["epoch"], 1);

    // The runner (fenced by the lease) commits v2 at the epoch; the notes it
    // queued carry the same lease.
    wait_for("the final pass", 60, || {
        store.get_meeting(&meeting).unwrap().transcript_version > live_version
    });
    wait_for("the notes", 60, || {
        !store.note_blocks(&meeting).unwrap().is_empty()
    });
    let notes_job = store
        .jobs_for_meeting(&meeting)
        .unwrap()
        .into_iter()
        .find(|j| j.kind == NOTES_FINAL_JOB)
        .expect("notes_final queued");
    assert_eq!(notes_job.payload["lease"], lease_id.as_str());
    let lease = store.lease_state(&lease_id).unwrap().unwrap();
    assert_eq!(lease.state, "done", "closed by the commit");
    assert!(lease.progress > 0.0, "the phone's chip has a percentage");

    // The phone's next session sees the lease done and pulls the new version.
    let (mine, _) = sync(&hub, &spoke);
    assert!(
        mine.lease_infos.iter().any(|i| i.state == "done"),
        "{:?}",
        mine.lease_infos
    );
    let on_phone = spoke.store.get_meeting(&meeting).unwrap();
    assert_eq!(
        on_phone.transcript_version,
        store.get_meeting(&meeting).unwrap().transcript_version
    );
    assert_eq!(texts(&spoke.store, &meeting), texts(&store, &meeting));
    assert!(!spoke.store.note_blocks(&meeting).unwrap().is_empty());
    let own = spoke.store.leases_for_meeting(&meeting).unwrap();
    assert_eq!(own[0].state, "done");
}

#[test]
fn the_listener_exists_only_while_enabled_paired_and_unlocked() {
    let _turn = NET.lock().unwrap_or_else(|e| e.into_inner());
    let lan = ghi_net::lan::lan_addrs();
    let hub = hub_with(lan.clone(), REACHABLE_WITHIN);
    hub.svc.start();
    let base = ghi_net::lan::listeners_open();

    // On, nobody paired, nothing open: no listener.
    hub.svc.set_enabled(true).unwrap();
    assert!(!hub.svc.listening());
    assert_eq!(ghi_net::lan::listeners_open(), base);

    pair(&hub, &spoke());
    if lan.is_empty() {
        eprintln!("no private network address here: the socket part is skipped");
        hub.svc.stop();
        return;
    }
    wait_for("the listener", 5, || hub.svc.listening());
    assert!(ghi_net::lan::listeners_open() > base);

    // The app lock closes it (and unlocking brings it back, still paired).
    hub.core.set_locked(true);
    wait_for("the listener to close", 5, || {
        !hub.svc.listening() && ghi_net::lan::listeners_open() == base
    });
    hub.core.set_locked(false);
    wait_for("the listener to reopen", 5, || hub.svc.listening());

    // Off: closed again.
    hub.svc.set_enabled(false).unwrap();
    assert!(!hub.svc.listening());
    assert_eq!(ghi_net::lan::listeners_open(), base);
    hub.svc.stop();
    assert_eq!(ghi_net::lan::listeners_open(), base);
}

#[test]
fn pairing_alone_opens_the_listener_and_closing_the_sheet_closes_it() {
    let _turn = NET.lock().unwrap_or_else(|e| e.into_inner());
    let lan = ghi_net::lan::lan_addrs();
    if lan.is_empty() {
        eprintln!("no private network address here: skipped");
        return;
    }
    let hub = hub_with(lan, REACHABLE_WITHIN);
    let base = ghi_net::lan::listeners_open();
    assert_eq!(hub.svc.pair_open().unwrap_err(), ERR_OFF, "sync is off");
    hub.svc.set_enabled(true).unwrap();
    let offer = hub.svc.pair_open().unwrap();
    assert!(offer.qr_svg.starts_with("<?xml") || offer.qr_svg.contains("<svg"));
    assert_eq!(offer.expires_ms, PAIR_CODE_TTL_MS);
    assert!(hub.svc.listening());
    assert!(ghi_net::lan::listeners_open() > base);
    hub.svc.pair_close().unwrap();
    assert!(!hub.svc.listening());
    assert_eq!(ghi_net::lan::listeners_open(), base);
    hub.svc.stop();
}

#[test]
fn delete_everything_with_an_unreachable_device_does_not_wait() {
    let hub = hub_with(Vec::new(), Duration::ZERO);
    let phone = spoke();
    pair(&hub, &phone);
    let started = Instant::now();
    hub.core
        .delete_everything_with(&|| hub.svc.delete_everywhere_prepare(true))
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(30));
    let status = hub.svc.delete_everywhere_status();
    assert_eq!(status.state, DeleteEverywhereState::Done);
    assert_eq!(
        status.waiting_for,
        vec!["iPhone".to_string()],
        "not reached"
    );
    // The identity went with the data.
    let secrets = hub.core.secrets().unwrap();
    assert!(Identity::load(secrets.as_ref()).unwrap().is_none());
    assert!(!hub.svc.listening());
}

#[test]
fn delete_everything_waits_for_a_nearby_device_until_the_user_skips() {
    let hub = hub_with(Vec::new(), Duration::from_secs(3600));
    let phone = spoke();
    pair(&hub, &phone);
    sync(&hub, &phone); // seen just now
    let (core, svc) = (hub.core.clone(), hub.svc.clone());
    let deleting =
        thread::spawn(move || core.delete_everything_with(&|| svc.delete_everywhere_prepare(true)));
    wait_for("the wait to show", 10, || {
        hub.svc.delete_everywhere_status().state == DeleteEverywhereState::Waiting
    });
    assert_eq!(
        hub.svc.delete_everywhere_status().waiting_for,
        vec!["iPhone".to_string()]
    );
    assert!(!deleting.is_finished(), "still waiting for the phone");
    hub.svc.delete_everywhere_skip();
    deleting.join().unwrap().unwrap();
    assert_eq!(
        hub.svc.delete_everywhere_status().state,
        DeleteEverywhereState::Done
    );
    let secrets = hub.core.secrets().unwrap();
    assert!(Identity::load(secrets.as_ref()).unwrap().is_none());
}

#[test]
fn a_nearby_device_that_connects_takes_its_wipe_and_the_wait_ends() {
    let hub = hub_with(Vec::new(), Duration::from_secs(3600));
    let phone = spoke();
    pair(&hub, &phone);
    let meeting = record_on_spoke(&phone);
    sync(&hub, &phone);
    sync(&hub, &phone);
    let (core, svc) = (hub.core.clone(), hub.svc.clone());
    let deleting =
        thread::spawn(move || core.delete_everything_with(&|| svc.delete_everywhere_prepare(true)));
    wait_for("the wait to show", 10, || {
        hub.svc.delete_everywhere_status().state == DeleteEverywhereState::Waiting
    });
    // The phone's next session delivers the wipe.
    let node = hub.svc.node().unwrap();
    let (a, b) = mem_pipe();
    let svc = hub.svc.clone();
    let server = thread::spawn(move || svc.serve_connection(&node, a, None, None));
    let mine = session_over(
        phone.store.clone() as Arc<dyn SyncStore>,
        Arc::new(SystemClock),
        &phone.identity,
        b,
    )
    .unwrap();
    assert_eq!(mine.closed_by, Some(ControlOutcome::Wiped));
    server.join().unwrap().unwrap();
    deleting.join().unwrap().unwrap();
    // The phone shredded what it had exchanged with the desktop, and let go.
    assert!(phone.store.get_meeting(&meeting).is_err());
    assert!(phone.store.devices().unwrap().is_empty());
    assert!(
        hub.events()
            .iter()
            .any(|e| matches!(e, SyncEvent::WipeDone { .. }))
    );
}

#[test]
fn unpair_and_wipe_delivers_the_wipe_and_plain_unpair_refuses_the_phone() {
    let hub = hub();
    let (a, b) = (spoke(), spoke());
    pair(&hub, &a);
    let gid_a = a.identity.device_gid.clone();
    hub.svc.unpair_and_wipe(&gid_a).unwrap();
    assert_eq!(
        hub.svc.devices().unwrap()[0].state,
        DeviceState::WipePending
    );
    let node = hub.svc.node().unwrap();
    let (x, y) = mem_pipe();
    let svc = hub.svc.clone();
    let server = thread::spawn(move || svc.serve_connection(&node, x, None, None));
    let report = session_over(
        a.store.clone() as Arc<dyn SyncStore>,
        Arc::new(SystemClock),
        &a.identity,
        y,
    )
    .unwrap();
    assert_eq!(report.closed_by, Some(ControlOutcome::Wiped));
    server.join().unwrap().unwrap();
    assert!(hub.svc.devices().unwrap().is_empty());
    assert!(
        hub.events()
            .iter()
            .any(|e| matches!(e, SyncEvent::WipeDone { gid } if *gid == gid_a))
    );

    // A second phone, plainly unpaired: it is told on its next session.
    pair(&hub, &b);
    hub.svc.unpair(&b.identity.device_gid).unwrap();
    assert_eq!(
        hub.svc.devices().unwrap()[0].state,
        DeviceState::UnpairPending
    );

    // A key the hub never paired is still refused (nothing is served to it
    // beyond the sheet's own node, whose code it does not know).
    let stranger = spoke();
    lock(&hub.svc.state).pairing_until = Some(Instant::now() + PAIR_TTL);
    hub.svc.reconcile();
    let node = hub.svc.node().expect("the sheet brings the node up");
    let (x, y) = mem_pipe();
    let svc = hub.svc.clone();
    let server = thread::spawn(move || svc.serve_connection(&node, x, None, None));
    let refused = session_over(
        stranger.store.clone() as Arc<dyn SyncStore>,
        Arc::new(SystemClock),
        &stranger.identity,
        y,
    );
    assert!(refused.is_err());
    assert!(server.join().unwrap().is_err());

    // The unpaired phone connects: it gets `Unpair` and the pin goes.
    let node = hub.svc.node().expect("still up");
    let (x, y) = mem_pipe();
    let svc = hub.svc.clone();
    let server = thread::spawn(move || svc.serve_connection(&node, x, None, None));
    let report = session_over(
        b.store.clone() as Arc<dyn SyncStore>,
        Arc::new(SystemClock),
        &b.identity,
        y,
    )
    .unwrap();
    assert_eq!(report.closed_by, Some(ControlOutcome::Unpaired));
    server.join().unwrap().unwrap();
    assert!(b.store.devices().unwrap().is_empty(), "the phone let go");
    assert!(hub.svc.devices().unwrap().is_empty());
    assert!(hub.events().iter().any(
        |e| matches!(e, SyncEvent::Unpaired { by_peer: false, name, .. } if name == "iPhone")
    ));
}

#[test]
fn the_listener_stays_open_for_an_unpair_the_phone_has_not_taken_yet() {
    let _turn = NET.lock().unwrap_or_else(|e| e.into_inner());
    let lan = ghi_net::lan::lan_addrs();
    if lan.is_empty() {
        eprintln!("no private network address here: skipped");
        return;
    }
    let hub = hub_with(lan, REACHABLE_WITHIN);
    hub.svc.start();
    let base = ghi_net::lan::listeners_open();
    hub.svc.set_enabled(true).unwrap();
    let phone = spoke();
    pair(&hub, &phone);
    wait_for("the listener", 5, || hub.svc.listening());

    hub.svc.unpair(&phone.identity.device_gid).unwrap();
    assert!(hub.svc.listening(), "the last pin is not gone yet");
    assert!(ghi_net::lan::listeners_open() > base);

    let node = hub.svc.node().unwrap();
    let (a, b) = mem_pipe();
    let svc = hub.svc.clone();
    let server = thread::spawn(move || svc.serve_connection(&node, a, None, None));
    let mine = session_over(
        phone.store.clone() as Arc<dyn SyncStore>,
        Arc::new(SystemClock),
        &phone.identity,
        b,
    )
    .unwrap();
    assert_eq!(mine.closed_by, Some(ControlOutcome::Unpaired));
    server.join().unwrap().unwrap();
    wait_for("the listener to close", 5, || {
        !hub.svc.listening() && ghi_net::lan::listeners_open() == base
    });
    hub.svc.stop();
}

#[test]
fn an_unpair_nobody_takes_is_dropped_when_its_wait_ends() {
    let hub = hub();
    let phone = spoke();
    pair(&hub, &phone);
    hub.svc.unpair(&phone.identity.device_gid).unwrap();
    let store = hub.store();
    assert!(store.expire_unpair_pending(i64::MAX).unwrap().is_empty());
    assert_eq!(store.devices().unwrap().len(), 1, "still waiting");
    assert_eq!(store.expire_unpair_pending(0).unwrap().len(), 1);
    assert!(store.devices().unwrap().is_empty());
}

#[test]
fn a_settings_patch_records_only_the_allowlisted_keys() {
    let hub = hub();
    let store = hub.store();
    let patch = crate::system::SettingsPatch {
        meeting_language: Some(crate::system::MeetingLanguage::Vi),
        strict_offline: Some(true),
        ..Default::default()
    };
    crate::system::patch_settings_core(&hub.core, patch).unwrap();
    let keys: Vec<String> = store
        .changes_since(0, 256)
        .unwrap()
        .changes
        .into_iter()
        .filter_map(|c| match c.record {
            Record::Setting(r) => Some(r.key),
            _ => None,
        })
        .collect();
    assert_eq!(
        keys,
        vec!["meetingLanguage".to_string()],
        "not strictOffline, not `app`"
    );

    // The vocabulary too.
    crate::settings_cmd::store_terms(&store, vec!["Ghira".into(), "Vĩnh".into()]).unwrap();
    let keys: Vec<String> = store
        .changes_since(0, 256)
        .unwrap()
        .changes
        .into_iter()
        .filter_map(|c| match c.record {
            Record::Setting(r) => Some(r.key),
            _ => None,
        })
        .collect();
    assert!(keys.contains(&"vocabulary".to_string()), "{keys:?}");
}

#[test]
fn a_setting_from_the_phone_reloads_the_cache_and_tells_the_app() {
    let (hub, phone) = (hub(), spoke());
    pair(&hub, &phone);
    // Warm the cache with the current value, then send the opposite.
    let was = crate::system::load_settings(&hub.core)
        .unwrap()
        .cloud_redact;
    let before = hub.settings_calls.load(Ordering::SeqCst);
    phone
        .store
        .put_synced("cloudRedact", &(!was).to_string())
        .unwrap();
    phone.store.put_synced("audioRetentionDays", "30").unwrap();
    sync(&hub, &phone);
    let s = crate::system::load_settings(&hub.core).unwrap();
    assert_eq!(s.cloud_redact, !was, "the cache was reloaded");
    assert_eq!(s.audio_retention_days, 30);
    assert!(hub.settings_calls.load(Ordering::SeqCst) > before);
}

#[test]
fn a_mass_delete_waits_for_the_user_and_applies_after_accept() {
    let (hub, phone) = (hub(), spoke());
    pair(&hub, &phone);
    let mut gids = Vec::new();
    for i in 0..12 {
        let m = phone
            .store
            .create_meeting(NewMeeting {
                title: format!("m{i}"),
                ..Default::default()
            })
            .unwrap();
        phone
            .store
            .add_segment(
                &m.gid,
                NewSegment {
                    t0_ms: 0,
                    t1_ms: 900,
                    text: format!("line {i}"),
                    ..Default::default()
                },
            )
            .unwrap();
        phone.store.finish_meeting(&m.gid, 1_000).unwrap();
        gids.push(m.gid);
    }
    sync(&hub, &phone);
    sync(&hub, &phone);
    let store = hub.store();
    assert!(gids.iter().all(|g| store.get_meeting(g).is_ok()));
    for g in &gids {
        phone.store.delete_meeting(g).unwrap();
    }
    // A new meeting made meanwhile still syncs: only the delete batch waits.
    let fresh = phone
        .store
        .create_meeting(NewMeeting {
            title: "fresh".into(),
            ..Default::default()
        })
        .unwrap();
    phone.store.finish_meeting(&fresh.gid, 1_000).unwrap();
    let (mine, theirs) = sync_raw(&hub, &phone);
    assert!(mine.unwrap().tombs_held >= gids.len());
    let Ok(Served::Session(theirs)) = theirs else {
        panic!("the hub ends the session cleanly");
    };
    assert!(theirs.needs_confirm > 0, "{theirs:?}");
    assert!(
        hub.events()
            .iter()
            .any(|e| matches!(e, SyncEvent::NeedsConfirm { device, .. } if device == "iPhone")),
        "{:?}",
        hub.events()
    );
    assert!(
        gids.iter().all(|g| store.get_meeting(g).is_ok()),
        "nothing applied yet"
    );
    assert!(store.get_meeting(&fresh.gid).is_ok(), "rows keep syncing");

    hub.svc.confirm_mass_delete(true).unwrap();
    assert_eq!(
        hub.svc.confirm_mass_delete(true).unwrap_err(),
        ERR_NOTHING_TO_CONFIRM
    );
    let (mine, theirs) = sync_raw(&hub, &phone);
    mine.unwrap();
    theirs.unwrap();
    assert!(
        gids.iter().all(|g| store.get_meeting(g).is_err()),
        "deleted after the answer"
    );
}

#[test]
fn a_store_restored_with_a_new_device_gid_drops_its_old_pins_and_identity() {
    let (hub, phone) = (hub(), spoke());
    pair(&hub, &phone);
    assert_eq!(hub.svc.devices().unwrap().len(), 1);
    // Another gid than the store's: what an archive restore leaves behind.
    let secrets = hub.core.secrets().unwrap();
    Identity::delete(secrets.as_ref()).unwrap();
    Identity::load_or_create(secrets.as_ref(), &ghi_store::new_gid()).unwrap();
    hub.svc.set_enabled(true).unwrap();
    assert!(
        hub.svc.devices().unwrap().is_empty(),
        "the old pairing is unusable"
    );
    let id = Identity::load(secrets.as_ref()).unwrap().unwrap();
    assert_eq!(id.device_gid, hub.gid());
}

#[test]
fn a_stopped_service_lets_go_of_the_store() {
    let (hub, phone) = (hub(), spoke());
    let store = hub.store();
    let base = Arc::strong_count(&store);
    pair(&hub, &phone);
    assert!(Arc::strong_count(&store) > base, "the node serves it");
    hub.svc.stop();
    assert!(hub.svc.node().is_none());
    assert_eq!(
        Arc::strong_count(&store),
        base,
        "nothing of sync holds the store"
    );
}

#[test]
fn the_fence_follows_the_lease_and_ignores_unleased_jobs() {
    let hub = hub();
    let store = hub.store();
    let m = store
        .create_meeting(NewMeeting {
            title: "x".into(),
            ..Default::default()
        })
        .unwrap();
    let fence = lease_fence(store.clone());
    let job = |kind: &str, payload: serde_json::Value| Job {
        id: 1,
        meeting_gid: Some(m.gid.clone()),
        kind: kind.to_string(),
        state: ghi_store::jobs::JobState::Queued,
        progress: 0.0,
        attempts: 0,
        payload_version: 1,
        payload,
    };
    assert!(fence(
        &job(FINAL_PASS_JOB, serde_json::json!({})),
        FenceAt::Claim
    ));
    let leased = job(
        FINAL_PASS_JOB,
        serde_json::json!({"lease": "nope", "epoch": 1}),
    );
    assert!(
        !fence(&leased, FenceAt::Claim),
        "an unknown lease holds nothing"
    );

    let clock = SystemClock;
    let boot = clock.boot_id();
    let now = i64::try_from(clock.now_cont_ns()).unwrap();
    store
        .lease_open(&Lease {
            job_uuid: "l1".into(),
            meeting_gid: m.gid.clone(),
            role: LeaseRole::Holder,
            epoch: 1,
            kinds: vec![FINAL_PASS_JOB.into()],
            state: "granted".into(),
            ttl_ms: 120_000,
            // 90 s of lease left: the claim passes, the commit (60 s margin) too,
            // but not once it is under a minute.
            deadline_cont_ns: Some(now + 90_000_000_000),
            boot_id: Some(boot.clone()),
            wall_deadline_ms: None,
            progress: 0.0,
            peer_gid: None,
        })
        .unwrap();
    let l1 = job(
        FINAL_PASS_JOB,
        serde_json::json!({"lease": "l1", "epoch": 1}),
    );
    assert!(fence(&l1, FenceAt::Claim));
    assert!(fence(&l1, FenceAt::Commit));
    store
        .lease_renew("l1", 120_000, now + 30_000_000_000, &boot)
        .unwrap();
    assert!(fence(&l1, FenceAt::Checkpoint), "30 s left still runs");
    assert!(
        !fence(&l1, FenceAt::Commit),
        "but is inside the commit margin"
    );
    // Another boot: suspended until renewed.
    store
        .lease_renew("l1", 120_000, now + 90_000_000_000, "other-boot")
        .unwrap();
    assert!(!fence(&l1, FenceAt::Claim));
}

#[test]
fn a_local_delete_is_pending_until_the_phone_has_it_then_progress_says_zero() {
    let (hub, phone) = (hub(), spoke());
    pair(&hub, &phone);
    let m = phone
        .store
        .create_meeting(NewMeeting {
            title: "to delete".into(),
            ..Default::default()
        })
        .unwrap();
    phone.store.finish_meeting(&m.gid, 1_000).unwrap();
    sync(&hub, &phone);
    sync(&hub, &phone);
    let store = hub.store();
    assert!(store.get_meeting(&m.gid).is_ok());
    store.delete_meeting(&m.gid).unwrap();
    assert!(
        pending_changes(&store) > 0,
        "the phone has not seen the delete"
    );
    sync(&hub, &phone);
    sync(&hub, &phone);
    assert!(
        phone.store.get_meeting(&m.gid).is_err(),
        "deleted on the phone"
    );
    assert_eq!(pending_changes(&store), 0);
    let last = hub.events().into_iter().rev().find_map(|e| match e {
        SyncEvent::Progress { pending } => Some(pending),
        _ => None,
    });
    assert_eq!(last, Some(0), "the UI's \"deleted on all devices\"");
}

#[test]
fn three_failed_pairings_tell_the_sheet_to_show_a_new_code() {
    use ghi_sync::transport::{NoiseTransport, Transport};
    let hub = hub();
    hub.svc.set_enabled(true).unwrap();
    lock(&hub.svc.state).pairing_until = Some(Instant::now() + PAIR_TTL);
    hub.svc.reconcile();
    let node = hub.svc.node().expect("the sheet brings the node up");
    let qr = ghi_sync::qr::parse(&node.open_pairing_code(&[]).unwrap()).unwrap();
    let spent = |hub: &Hub| {
        hub.events()
            .iter()
            .filter(|e| matches!(e, SyncEvent::PairCodeSpent))
            .count()
    };
    for n in 1..=3 {
        let (a, b) = mem_pipe();
        let (svc, nd) = (hub.svc.clone(), node.clone());
        let server = thread::spawn(move || svc.serve_connection(&nd, a, None, None));
        let me = Identity::generate().unwrap();
        let mut t =
            NoiseTransport::initiate(b, &me.secret, &qr.pk, &qr.psk, &ghi_sync::wire::MAJORS)
                .unwrap();
        t.send(b"nonsense").unwrap();
        drop(t);
        assert!(server.join().unwrap().is_err());
        assert_eq!(spent(&hub), usize::from(n == 3), "after {n}");
    }
    assert!(!node.pairing_open(), "the dead code is dropped");
}

#[test]
fn a_new_session_of_a_device_replaces_its_stale_one() {
    use ghi_sync::service::initiate_hub;
    use ghi_sync::session::spoke::SpokeSession;
    let _turn = NET.lock().unwrap_or_else(|e| e.into_inner());
    let lan = ghi_net::lan::lan_addrs();
    if lan.is_empty() {
        eprintln!("no private network address here: skipped");
        return;
    }
    let hub = hub_with(lan, REACHABLE_WITHIN);
    hub.svc.start();
    hub.svc.set_enabled(true).unwrap();
    let phone = spoke();
    pair(&hub, &phone);
    wait_for("the listener", 5, || hub.svc.listening());
    let addr = loop {
        let found = lock(&hub.svc.state)
            .running
            .as_ref()
            .and_then(|r| lock(&r.addrs).first().copied());
        if let Some(a) = found {
            break a;
        }
        thread::sleep(Duration::from_millis(50));
    };

    let open = |phone: &Spoke| {
        let stream = ghi_net::lan::connect(addr, ghi_net::lan::CONNECT_TIMEOUT).unwrap();
        let store = phone.store.clone() as Arc<dyn SyncStore>;
        let (t, hub_dev) = initiate_hub(store.as_ref(), &phone.identity, stream).unwrap();
        let mut s = SpokeSession::new(store, Arc::new(SystemClock), t, hub_dev.gid);
        s.run_once().unwrap();
        s
    };
    let mut old = open(&phone);
    assert!(!old.ping().unwrap());
    // The phone reconnects (it never said Bye): the hub drops the stale one.
    let mut new = open(&phone);
    wait_for("the old session to end", 5, || old.ping().is_err());
    assert!(new.ping().is_ok(), "the new session lives");
    assert_eq!(
        lock(&hub.svc.sessions)
            .values()
            .filter(|e| e.device.is_some())
            .count(),
        1
    );
    hub.svc.stop();
}
