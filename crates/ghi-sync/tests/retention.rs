// SPDX-License-Identifier: Apache-2.0
//! Retention propagation (doc 07 §7.8 and §11; 15-L): audio removed by
//! retention on one device is removed on the others (track tombstones) while
//! the text stays; a sweep never touches a meeting with an open lease.

mod common;

use common::*;
use ghi_store::bundle::part_path;
use ghi_store::store::TrackKind;
use ghi_store::sync::leases::{Lease, LeaseRole};
use ghi_store::tombstones::Tombstone;

const PAGES: usize = 12;

fn track_tombstones(n: &Node) -> Vec<Tombstone> {
    n.store()
        .tombstones_since(0)
        .unwrap()
        .into_iter()
        .filter(|t| t.kind == "track")
        .collect()
}

/// Anything of the meeting's audio on disk: files in its bundle directory
/// (an empty directory is no audio).
fn has_audio_files(n: &Node, meeting: &str) -> bool {
    let dir = n.data_dir().join("bundles").join(meeting);
    std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_some())
}

struct Rig {
    hub_n: Node,
    phone: Node,
    meeting: String,
}

/// A paired hub and phone; the phone's meeting with audio and text has
/// reached the hub, audio included.
fn rig() -> Rig {
    let (hub_n, phone) = (node(), node());
    let hub = hub_n.hub();
    pair(&hub, &hub_n, &phone);
    let meeting = meeting_with_audio(phone.store(), "With audio", PAGES);
    phone
        .store()
        .add_segment(&meeting, seg("A line that stays", 1_000))
        .unwrap();
    sync(&hub, &phone);
    sync(&hub, &phone);
    for n in [&hub_n, &phone] {
        assert_eq!(
            n.store().tracks(&meeting).unwrap(),
            vec![(TrackKind::Mic, PAGES as u32)]
        );
    }
    Rig {
        hub_n,
        phone,
        meeting,
    }
}

fn assert_text_stays(r: &Rig) {
    for n in [&r.hub_n, &r.phone] {
        assert_eq!(
            texts(n.store(), &r.meeting),
            ["hello there", "A line that stays"]
        );
        assert_eq!(
            n.store().get_meeting(&r.meeting).unwrap().title,
            "With audio"
        );
        assert!(!n.store().audio_available(&r.meeting).unwrap());
    }
}

#[test]
fn retention_on_the_desktop_removes_the_audio_on_the_phone_and_keeps_the_text() {
    let r = rig();
    let hub = r.hub_n.hub();
    assert_eq!(r.hub_n.store().delete_audio(&r.meeting).unwrap(), 1);
    assert!(
        r.phone.store().audio_available(&r.meeting).unwrap(),
        "not yet"
    );
    sync(&hub, &r.phone);
    for n in [&r.hub_n, &r.phone] {
        assert!(n.store().tracks(&r.meeting).unwrap().is_empty());
        assert!(!has_audio_files(n, &r.meeting), "the audio files are gone");
        assert_eq!(track_tombstones(n).len(), 1, "one track tombstone");
    }
    assert_text_stays(&r);
    // The tombstone is the same gid on both, and nothing comes back.
    assert_eq!(
        track_tombstones(&r.hub_n)[0].gid,
        track_tombstones(&r.phone)[0].gid
    );
    for _ in 0..2 {
        let (m, t) = sync(&hub, &r.phone);
        assert_eq!(m.tracks_sent, 0);
        assert!(t.tracks_received.is_empty());
    }
    sync(&hub, &r.phone);
    let (m, t) = sync(&hub, &r.phone);
    assert!(idle(&m) && idle(&t), "{m:?} {t:?}");
    assert!(r.hub_n.store().tracks(&r.meeting).unwrap().is_empty());
}

#[test]
fn retention_on_the_phone_removes_the_audio_on_the_desktop_and_it_is_never_sent_again() {
    let r = rig();
    let hub = r.hub_n.hub();
    assert_eq!(r.phone.store().delete_audio(&r.meeting).unwrap(), 1);
    sync(&hub, &r.phone);
    sync(&hub, &r.phone);
    for n in [&r.hub_n, &r.phone] {
        assert!(n.store().tracks(&r.meeting).unwrap().is_empty());
        assert!(!has_audio_files(n, &r.meeting));
    }
    assert_text_stays(&r);
    let (m, t) = sync(&hub, &r.phone);
    assert!(idle(&m) && idle(&t), "{m:?} {t:?}");
}

#[test]
fn the_time_based_sweep_propagates_like_a_manual_cut() {
    let r = rig();
    let hub = r.hub_n.hub();
    // The policy reaches both devices as a meeting field; the sweep then runs
    // where it is due.
    r.phone
        .store()
        .set_audio_retained_until(&r.meeting, Some(1_000))
        .unwrap();
    sync(&hub, &r.phone);
    assert_eq!(
        r.hub_n
            .store()
            .get_meeting(&r.meeting)
            .unwrap()
            .audio_retained_until,
        Some(1_000),
        "the retention field synced"
    );
    let report = r.hub_n.store().retention_sweep(2_000).unwrap();
    assert_eq!((report.meetings, report.tracks), (1, 1));
    sync(&hub, &r.phone);
    sync(&hub, &r.phone);
    for n in [&r.hub_n, &r.phone] {
        assert!(n.store().tracks(&r.meeting).unwrap().is_empty());
    }
    assert_text_stays(&r);
    // The phone's own sweep finds nothing left to do.
    let report = r.phone.store().retention_sweep(2_000).unwrap();
    assert_eq!((report.meetings, report.tracks), (0, 0));
}

fn open_lease(n: &Node, meeting: &str, role: LeaseRole, state: &str, job: &str) {
    n.store()
        .lease_open(&Lease {
            job_uuid: job.into(),
            meeting_gid: meeting.into(),
            role,
            epoch: 1,
            kinds: vec!["final_pass".into()],
            state: state.into(),
            ttl_ms: 3_600_000,
            deadline_cont_ns: None,
            boot_id: None,
            wall_deadline_ms: None,
            progress: 0.0,
            peer_gid: None,
        })
        .unwrap();
}

#[test]
fn a_sweep_skips_a_meeting_with_an_open_lease_until_the_lease_closes() {
    for (role, open_state) in [
        (LeaseRole::Holder, "granted"),
        (LeaseRole::Holder, "running"),
        (LeaseRole::Grantor, "offered"),
        (LeaseRole::Grantor, "revoking"),
    ] {
        let r = rig();
        r.hub_n
            .store()
            .set_audio_retained_until(&r.meeting, Some(1_000))
            .unwrap();
        open_lease(&r.hub_n, &r.meeting, role, open_state, "job-1");
        let report = r.hub_n.store().retention_sweep(2_000).unwrap();
        assert_eq!(
            (report.meetings, report.tracks),
            (0, 0),
            "{role:?}/{open_state}: the sweep cut a leased meeting's audio"
        );
        assert_eq!(r.hub_n.store().tracks(&r.meeting).unwrap().len(), 1);
        assert!(has_audio_files(&r.hub_n, &r.meeting));
        assert!(track_tombstones(&r.hub_n).is_empty());

        // The lease closes: the next sweep takes the audio.
        assert!(
            r.hub_n
                .store()
                .lease_transition("job-1", &[open_state], "done")
                .unwrap()
        );
        let report = r.hub_n.store().retention_sweep(2_000).unwrap();
        assert_eq!(
            (report.meetings, report.tracks),
            (1, 1),
            "{role:?}/{open_state}"
        );
        assert!(r.hub_n.store().tracks(&r.meeting).unwrap().is_empty());
    }
}

#[test]
fn a_cut_before_the_audio_travelled_means_it_never_does() {
    let (hub_n, phone) = (node(), node());
    let hub = hub_n.hub();
    pair(&hub, &hub_n, &phone);
    let m = meeting_with_audio(phone.store(), "Cut early", PAGES);
    phone.store().delete_audio(&m).unwrap();
    sync(&hub, &phone);
    sync(&hub, &phone);
    assert!(hub_n.store().tracks(&m).unwrap().is_empty());
    assert!(!has_audio_files(&hub_n, &m));
    assert_eq!(texts(hub_n.store(), &m), ["hello there"]);
    let (a, b) = sync(&hub, &phone);
    assert!(idle(&a) && idle(&b), "{a:?} {b:?}");
}

#[test]
fn a_cut_while_the_desktop_holds_half_of_the_track_leaves_no_audio_bytes_behind() {
    // A long track whose transfer is cut half way, then the phone's audio is
    // removed by retention: the desktop's `.part` must not outlive it.
    let (mut hub_n, phone) = (node(), node());
    let hub = hub_n.hub();
    pair(&hub, &hub_n, &phone);
    drop(hub);
    let m = phone
        .store()
        .create_meeting(ghi_store::store::NewMeeting {
            title: "Half way".into(),
            ..Default::default()
        })
        .unwrap();
    record_track_sized(phone.store(), &m.gid, TrackKind::Mic, 160, 20_000);
    phone.store().add_segment(&m.gid, seg("kept", 0)).unwrap();
    phone.store().finish_meeting(&m.gid, 60_000).unwrap();
    // Cut sessions until the desktop holds a `.part`.
    let calibrate = {
        let (n2, p2) = (node(), node());
        let h = n2.hub();
        pair(&h, &n2, &p2);
        let m2 = p2
            .store()
            .create_meeting(ghi_store::store::NewMeeting::default())
            .unwrap();
        record_track_sized(p2.store(), &m2.gid, TrackKind::Mic, 160, 20_000);
        p2.store().add_segment(&m2.gid, seg("x", 0)).unwrap();
        p2.store().finish_meeting(&m2.gid, 1).unwrap();
        let ran = try_sync(&h, &p2, None, false);
        let frames = ran.spoke_frames;
        ran.ok();
        frames
    };
    let dest = hub_n.store().bundle_path(&m.gid, TrackKind::Mic).unwrap();
    let mut cut = calibrate / 2;
    loop {
        let ran = try_sync(&hub_n.hub(), &phone, Some(cut), false);
        assert!(ran.spoke.is_err());
        hub_n.reopen();
        if std::fs::metadata(part_path(&dest))
            .map(|x| x.len())
            .unwrap_or(0)
            > 50_000
        {
            break;
        }
        cut += 3;
        assert!(cut < calibrate);
    }

    phone.store().delete_audio(&m.gid).unwrap();
    let hub = hub_n.hub();
    sync(&hub, &phone);
    sync(&hub, &phone);
    assert!(hub_n.store().tracks(&m.gid).unwrap().is_empty());
    assert!(!dest.exists());
    assert!(
        !part_path(&dest).exists(),
        "a half-received encrypted track stays on the desktop after retention removed it"
    );
    assert!(
        !has_audio_files(&hub_n, &m.gid),
        "audio bytes of a cut meeting remain in its bundle directory"
    );
    assert_eq!(texts(hub_n.store(), &m.gid), ["kept"]);
}
