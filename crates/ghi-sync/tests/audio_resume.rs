// SPDX-License-Identifier: Apache-2.0
//! Audio resume (doc 07 §7.7 and §11; 15-L): a phone's track reaches the
//! desktop through connections that drop at random points and a receiver that
//! restarts (the store is closed and opened again); the finished bundle is
//! byte-identical. A corrupted page is caught by the receiver and never
//! lands in the bundle.

mod common;

use std::sync::Arc;

use common::*;
use ghi_store::bundle::part_path;
use ghi_store::store::TrackKind;
use ghi_sync::service::HubNode;

const PAGES: usize = 160;
const PAGE_BYTES: usize = 20_000;

struct Rig {
    hub_n: Node,
    phone: Node,
    meeting: String,
}

impl Rig {
    fn new() -> Rig {
        let (hub_n, phone) = (node(), node());
        let hub = hub_n.hub();
        pair(&hub, &hub_n, &phone);
        drop(hub);
        let m = phone
            .store()
            .create_meeting(ghi_store::store::NewMeeting {
                title: "Long call".into(),
                ..Default::default()
            })
            .unwrap();
        record_track_sized(phone.store(), &m.gid, TrackKind::Mic, PAGES, PAGE_BYTES);
        phone.store().add_segment(&m.gid, seg("hello", 0)).unwrap();
        phone.store().finish_meeting(&m.gid, 60_000).unwrap();
        Rig {
            hub_n,
            phone,
            meeting: m.gid,
        }
    }

    fn hub(&self) -> Arc<HubNode> {
        self.hub_n.hub()
    }

    fn dest(&self) -> std::path::PathBuf {
        self.hub_n
            .store()
            .bundle_path(&self.meeting, TrackKind::Mic)
            .unwrap()
    }

    fn source(&self) -> std::path::PathBuf {
        self.phone
            .store()
            .bundle_path(&self.meeting, TrackKind::Mic)
            .unwrap()
    }

    fn part_len(&self) -> Option<u64> {
        std::fs::metadata(part_path(&self.dest()))
            .ok()
            .map(|m| m.len())
    }

    fn complete_on_hub(&self) -> bool {
        self.dest().exists()
    }

    /// The receiver restarts.
    fn restart_hub(&mut self) {
        self.hub_n.reopen();
    }

    fn assert_identical(&self) {
        assert!(self.complete_on_hub(), "no bundle on the desktop");
        assert_eq!(read(&self.dest()), read(&self.source()), "bundle differs");
        assert!(self.part_len().is_none(), "a .part is left behind");
        let r = self
            .hub_n
            .store()
            .open_bundle(&self.meeting, TrackKind::Mic)
            .unwrap();
        assert_eq!(r.page_count() as usize, PAGES);
        assert!(r.complete());
        let s = self
            .phone
            .store()
            .open_bundle(&self.meeting, TrackKind::Mic)
            .unwrap();
        for i in [0, 1, PAGES as u32 / 2, PAGES as u32 - 1] {
            assert_eq!(r.page(i).unwrap(), s.page(i).unwrap(), "page {i}");
        }
    }
}

/// Frames a whole clean session puts on the wire, from each end.
fn calibrate() -> (usize, usize) {
    let rig = Rig::new();
    let ran = try_sync(&rig.hub(), &rig.phone, None, false);
    let (frames_spoke, frames_hub) = (ran.spoke_frames, ran.hub_frames);
    ran.ok();
    rig.assert_identical();
    (frames_spoke, frames_hub)
}

/// A small deterministic generator (the runs are replayable).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % below
    }
}

#[test]
fn drops_at_random_points_and_receiver_restarts_end_in_a_byte_identical_bundle() {
    let (spoke_frames, hub_frames) = calibrate();
    assert!(
        spoke_frames > 40,
        "the transfer spans many frames: {spoke_frames}"
    );

    let (mut partials, mut attempts_total) = (0, 0);
    for seed in 1..=6u64 {
        let mut rng = Lcg(seed);
        let mut rig = Rig::new();
        let mut last_part = 0u64;
        let mut attempts = 0;
        while !rig.complete_on_hub() {
            attempts += 1;
            assert!(attempts <= 120, "seed {seed}: the transfer never finished");
            // Cut the spoke or the hub at a random frame.
            let (cut, by_hub) = if rng.next(2) == 0 {
                (1 + rng.next(spoke_frames), false)
            } else {
                (1 + rng.next(hub_frames), true)
            };
            let ran = try_sync(&rig.hub(), &rig.phone, Some(cut), by_hub);
            if ran.spoke.is_ok() && ran.hub.is_ok() {
                // The cut fell after the end: a whole session.
            } else {
                rig.restart_hub();
            }
            if let Some(len) = rig.part_len() {
                assert!(
                    len >= last_part,
                    "seed {seed}: the .part shrank from {last_part} to {len} without corruption"
                );
                if len > 0 && len > last_part {
                    partials += 1;
                }
                last_part = len;
            }
            // Never a half-written final bundle: it exists whole or not at all.
            if rig.complete_on_hub() {
                rig.assert_identical();
            }
        }
        attempts_total += attempts;
        // Sessions settle (rows the cuts left in flight come round once; a
        // session cut after the last page is acked by a `TrackHave{complete}`
        // once), and no page ever travels again.
        let mut quiet = false;
        for _ in 0..3 {
            let (m, t) = sync(&rig.hub(), &rig.phone);
            assert!(
                t.tracks_received.is_empty(),
                "seed {seed}: pages travelled again"
            );
            if idle(&m) && idle(&t) {
                quiet = true;
                break;
            }
        }
        assert!(quiet, "seed {seed}: sessions never went idle");
        rig.assert_identical();
    }
    assert!(
        partials >= 3,
        "the random cuts never left a partial transfer to resume ({partials})"
    );
    assert!(attempts_total > 6, "no run needed a second attempt");
}

#[test]
fn a_cut_leaves_a_part_and_the_next_session_sends_only_what_is_missing() {
    let (spoke_frames, _) = calibrate();
    let mut rig = Rig::new();
    // Find a cut that leaves some pages on the desktop.
    let mut cut = spoke_frames / 2;
    let first = loop {
        let ran = try_sync(&rig.hub(), &rig.phone, Some(cut), false);
        assert!(ran.spoke.is_err(), "a cut at {cut} went unnoticed");
        rig.restart_hub();
        match rig.part_len() {
            Some(len) if len > 1_000 => break len,
            _ => {
                assert!(!rig.complete_on_hub());
                cut += 4;
                assert!(cut < spoke_frames, "no cut left a partial track");
            }
        }
    };
    // The resumed session ends the track, and puts fewer frames on the wire
    // than a whole one: it did not start over.
    let ran = try_sync(&rig.hub(), &rig.phone, None, false);
    let frames = ran.spoke_frames;
    ran.ok();
    rig.assert_identical();
    let whole = std::fs::metadata(rig.dest()).unwrap().len();
    assert!(first < whole);
    // Pages come in 64-page messages: a resumed run skips the verified ones.
    assert!(
        frames < spoke_frames,
        "{frames} frames to resume against {spoke_frames} for the whole track"
    );
}

#[test]
fn a_corrupted_page_in_the_senders_file_is_caught_and_the_repaired_file_resumes() {
    let mut rig = Rig::new();
    let src = rig.source();
    let good = read(&src);
    // Flip one byte deep inside the file (a page, not the header or footer).
    let at = good.len() * 6 / 10;
    let mut bad = good.clone();
    bad[at] ^= 0x20;
    std::fs::write(&src, &bad).unwrap();

    let ran = try_sync(&rig.hub(), &rig.phone, None, false);
    let (spoke_failed, hub_failed) = (ran.spoke.is_err(), ran.hub.is_err());
    // The corrupted page can never become part of the desktop's bundle.
    rig.restart_hub();
    if rig.complete_on_hub() {
        assert_eq!(
            read(&rig.dest()),
            good,
            "a corrupted bundle was accepted by the desktop"
        );
        panic!("the corrupted track was delivered whole (spoke_failed={spoke_failed})");
    }
    assert!(
        spoke_failed || hub_failed,
        "the corruption went unnoticed: no session error and no bundle"
    );
    let part = rig.part_len().unwrap_or(0);
    assert!(
        part < good.len() as u64 * 6 / 10 + 64,
        "the .part ({part} bytes) runs past the corrupted page"
    );

    // The file is repaired (restored); the next session resumes and ends.
    std::fs::write(&src, &good).unwrap();
    sync(&rig.hub(), &rig.phone);
    rig.assert_identical();
    assert_eq!(read(&rig.source()), good);
}

#[test]
fn a_corrupted_part_on_the_receiver_is_cut_back_and_the_bundle_still_ends_identical() {
    let (spoke_frames, _) = calibrate();
    let mut rig = Rig::new();
    let mut cut = spoke_frames * 2 / 3;
    loop {
        let ran = try_sync(&rig.hub(), &rig.phone, Some(cut), false);
        assert!(ran.spoke.is_err());
        rig.restart_hub();
        if rig.part_len().unwrap_or(0) > 100_000 {
            break;
        }
        cut += 3;
        assert!(cut < spoke_frames, "no cut left a long enough .part");
    }
    // A bit rots in the middle of what the desktop already holds.
    let part = part_path(&rig.dest());
    let mut bytes = read(&part);
    let at = bytes.len() / 2;
    bytes[at] ^= 0x01;
    std::fs::write(&part, &bytes).unwrap();
    rig.restart_hub();

    sync(&rig.hub(), &rig.phone);
    rig.assert_identical();
}

#[test]
fn a_part_of_a_track_the_phone_has_since_cut_restarts_from_nothing() {
    let (spoke_frames, _) = calibrate();
    let mut rig = Rig::new();
    let mut cut = spoke_frames / 2;
    loop {
        let ran = try_sync(&rig.hub(), &rig.phone, Some(cut), false);
        assert!(ran.spoke.is_err());
        rig.restart_hub();
        if rig.part_len().unwrap_or(0) > 100_000 {
            break;
        }
        cut += 3;
        assert!(cut < spoke_frames);
    }
    // The phone cuts its track (a discard): the audio is sealed again under a
    // new prefix, so the desktop's `.part` belongs to a track that no longer
    // exists.
    let kept = rig
        .phone
        .store()
        .truncate_track(&rig.meeting, TrackKind::Mic, 100)
        .unwrap();
    assert_eq!(kept, 100);
    sync(&rig.hub(), &rig.phone);
    assert!(rig.complete_on_hub());
    assert_eq!(read(&rig.dest()), read(&rig.source()));
    assert_eq!(
        rig.hub_n
            .store()
            .open_bundle(&rig.meeting, TrackKind::Mic)
            .unwrap()
            .page_count(),
        100
    );
    assert!(rig.part_len().is_none());
}
