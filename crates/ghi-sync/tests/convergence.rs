// SPDX-License-Identifier: Apache-2.0
//! Convergence (doc 07 §11; 15-L): one hub and two spokes on real stores, with
//! random local operations interleaved with random sync steps (complete
//! sessions, sessions cut after N frames on either end, a session run twice).
//! After quiescence every device holds the same state, no tombstoned gid is
//! alive, and every free-text value written is still current, kept as a
//! conflict copy, or under a tombstone.
//!
//! `PROPTEST_CASES` raises the number of random cases (default 32). The fixed
//! scenarios below pin the C2b rules the random walk may not reach.

mod common;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use common::*;
use ghi_store::store::{NewActionItem, NewSpeaker, Provenance, Store, TrackKind};
use ghi_sync::service::HubNode;
use proptest::prelude::*;

// ---------------------------------------------------------------- the world

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Segment,
    Note,
    Action,
    Speaker,
}

/// A free-text value some device wrote. Every text is unique in a run.
#[derive(Debug, Clone)]
struct Written {
    field: Field,
    meeting: String,
    gid: String,
    /// Another write on a device that held this value replaced it: not a
    /// concurrent loser, so it need not survive.
    overwritten: bool,
}

struct World {
    nodes: [Node; 3],
    hub: Arc<HubNode>,
    next: usize,
    created: usize,
    written: HashMap<String, Written>,
    /// Meetings whose audio some device cut.
    retained: BTreeSet<String>,
    /// Meetings recorded with audio, and where.
    audio_on: BTreeMap<String, usize>,
    applied: usize,
}

const MAX_MEETINGS: usize = 8;
const FOLDER_NAMES: [&str; 2] = ["Work", "Home"];
const TAG_NAMES: [&str; 3] = ["alpha", "beta", "gamma"];
const PEOPLE: [&str; 3] = ["Linh", "Minh", "An"];

impl World {
    fn new() -> World {
        let nodes = [node(), node(), node()];
        let hub = nodes[0].hub();
        pair(&hub, &nodes[0], &nodes[1]);
        pair(&hub, &nodes[0], &nodes[2]);
        World {
            nodes,
            hub,
            next: 0,
            created: 0,
            written: HashMap::new(),
            retained: BTreeSet::new(),
            audio_on: BTreeMap::new(),
            applied: 0,
        }
    }

    fn s(&self, dev: usize) -> &Store {
        self.nodes[dev].store()
    }

    /// A fresh text; never reused.
    fn text(&mut self, what: &str) -> String {
        self.next += 1;
        format!("{what} #{}", self.next)
    }

    fn note_written(&mut self, text: &str, field: Field, meeting: &str, gid: &str) {
        self.written.insert(
            text.to_string(),
            Written {
                field,
                meeting: meeting.to_string(),
                gid: gid.to_string(),
                overwritten: false,
            },
        );
    }

    /// The device is about to overwrite `old`: if it is one of ours, a later
    /// write took its place knowingly.
    fn overwrite(&mut self, old: &str) {
        if let Some(w) = self.written.get_mut(old) {
            w.overwritten = true;
        }
    }

    fn meetings(&self, dev: usize) -> Vec<String> {
        self.s(dev)
            .list_meetings(1000, 0)
            .unwrap()
            .into_iter()
            .map(|m| m.gid)
            .collect()
    }

    fn pick_meeting(&self, dev: usize, k: u8) -> Option<String> {
        let all = self.meetings(dev);
        (!all.is_empty()).then(|| all[k as usize % all.len()].clone())
    }

    // ------------------------------------------------------------- the ops

    fn create(&mut self, dev: usize, segs: u8, audio: bool, ai: u8) {
        if self.created >= MAX_MEETINGS {
            return;
        }
        self.created += 1;
        let title = self.text("title");
        let s = self.nodes[dev].store().clone();
        let m = s
            .create_meeting(ghi_store::store::NewMeeting {
                title: title.clone(),
                ..Default::default()
            })
            .unwrap();
        self.note_written(&title, Field::Title, &m.gid, &m.gid);
        let sp0 = s
            .add_speaker(
                &m.gid,
                NewSpeaker {
                    label_idx: 0,
                    color_slot: 0,
                    ..Default::default()
                },
            )
            .unwrap();
        s.add_speaker(
            &m.gid,
            NewSpeaker {
                label_idx: 1,
                color_slot: 1,
                ..Default::default()
            },
        )
        .unwrap();
        for i in 0..=segs {
            let t = self.text("line");
            let mut sg = seg(&t, i64::from(i) * 1000);
            sg.speaker_gid = Some(sp0.clone());
            let row = s.add_segment(&m.gid, sg).unwrap();
            self.note_written(&t, Field::Segment, &m.gid, &row.gid);
        }
        let body = self.text("note");
        let n = s
            .add_note_block(&m.gid, note(&body, Provenance::User))
            .unwrap();
        self.note_written(&body, Field::Note, &m.gid, &n.gid);
        for _ in 0..ai {
            let body = self.text("ai note");
            let n = s
                .add_note_block(&m.gid, note(&body, Provenance::Ai))
                .unwrap();
            self.note_written(&body, Field::Note, &m.gid, &n.gid);
        }
        if ai > 0 {
            let text = self.text("action");
            let a = s
                .add_action_item(
                    &m.gid,
                    NewActionItem {
                        text: text.clone(),
                        provenance: Provenance::Ai,
                        ..Default::default()
                    },
                )
                .unwrap();
            self.note_written(&text, Field::Action, &m.gid, &a.gid);
        }
        if audio {
            record_track(&s, &m.gid, TrackKind::Mic, 3);
            self.audio_on.insert(m.gid.clone(), dev);
        }
        s.finish_meeting(&m.gid, 4_000).unwrap();
        self.applied += 1;
    }

    fn edit_title(&mut self, dev: usize, k: u8) {
        let Some(m) = self.pick_meeting(dev, k) else {
            return;
        };
        let old = self.s(dev).get_meeting(&m).unwrap().title;
        self.overwrite(&old);
        let t = self.text("title");
        self.s(dev).set_meeting_title(&m, &t).unwrap();
        self.note_written(&t, Field::Title, &m, &m);
        self.applied += 1;
    }

    fn edit_note(&mut self, dev: usize, k: u8, j: u8) {
        let Some(m) = self.pick_meeting(dev, k) else {
            return;
        };
        let notes = self.s(dev).note_blocks(&m).unwrap();
        if notes.is_empty() {
            return;
        }
        let n = &notes[j as usize % notes.len()];
        self.overwrite(&n.body);
        let t = self.text("note");
        self.s(dev).update_note_block(&n.gid, &t).unwrap();
        self.note_written(&t, Field::Note, &m, &n.gid);
        self.applied += 1;
    }

    fn edit_segment(&mut self, dev: usize, k: u8, j: u8) {
        let Some(m) = self.pick_meeting(dev, k) else {
            return;
        };
        let segs = self.s(dev).segments(&m).unwrap();
        if segs.is_empty() {
            return;
        }
        let sg = &segs[j as usize % segs.len()];
        self.overwrite(&sg.text);
        let t = self.text("line");
        self.s(dev).update_segment_text(&sg.gid, &t).unwrap();
        self.note_written(&t, Field::Segment, &m, &sg.gid);
        self.applied += 1;
    }

    /// `shared`: a name from a small pool (two devices naming speakers alike
    /// make same-name persons); otherwise a unique name that must survive.
    fn rename(&mut self, dev: usize, k: u8, j: u8, shared: bool) {
        let Some(m) = self.pick_meeting(dev, k) else {
            return;
        };
        let speakers = self.s(dev).speakers(&m).unwrap();
        if speakers.is_empty() {
            return;
        }
        let sp = &speakers[j as usize % speakers.len()];
        if sp.merged_into.is_some() {
            return;
        }
        if let Some(old) = &sp.display_name {
            self.overwrite(old);
        }
        let name = if shared {
            PEOPLE[j as usize % PEOPLE.len()].to_string()
        } else {
            self.text("name")
        };
        if self.s(dev).rename_speaker(&sp.gid, Some(&name)).is_ok() {
            if !shared {
                self.note_written(&name, Field::Speaker, &m, &sp.gid);
            }
            self.applied += 1;
        }
    }

    fn merge(&mut self, dev: usize, k: u8, a: u8, b: u8) {
        let Some(m) = self.pick_meeting(dev, k) else {
            return;
        };
        // The app offers only speakers that are not merged away already (its
        // `merge_speakers_in` refuses the rest); two devices merging each way
        // still make a cycle.
        let speakers: Vec<_> = self
            .s(dev)
            .speakers(&m)
            .unwrap()
            .into_iter()
            .filter(|s| s.merged_into.is_none())
            .collect();
        if speakers.len() < 2 {
            return;
        }
        let from = &speakers[a as usize % speakers.len()].gid;
        let into = &speakers[b as usize % speakers.len()].gid;
        if self.s(dev).merge_speakers(from, into).is_ok() {
            self.applied += 1;
        }
    }

    fn folder(&mut self, dev: usize, k: u8) {
        if self
            .s(dev)
            .create_folder(FOLDER_NAMES[k as usize % FOLDER_NAMES.len()])
            .is_ok()
        {
            self.applied += 1;
        }
    }

    fn tag(&mut self, dev: usize, k: u8) {
        if self
            .s(dev)
            .create_tag(TAG_NAMES[k as usize % TAG_NAMES.len()])
            .is_ok()
        {
            self.applied += 1;
        }
    }

    fn tag_meeting(&mut self, dev: usize, k: u8, t: u8, remove: bool) {
        let Some(m) = self.pick_meeting(dev, k) else {
            return;
        };
        let tags = self.s(dev).tags().unwrap();
        if tags.is_empty() {
            return;
        }
        let tag = &tags[t as usize % tags.len()].gid;
        let r = if remove {
            self.s(dev).untag_meetings(&[m], tag)
        } else {
            self.s(dev).tag_meetings(&[m], tag)
        };
        if r.is_ok() {
            self.applied += 1;
        }
    }

    fn set_folder(&mut self, dev: usize, k: u8, f: u8) {
        let Some(m) = self.pick_meeting(dev, k) else {
            return;
        };
        let folders = self.s(dev).folders().unwrap();
        // One more choice than folders: out of any folder.
        let pick = f as usize % (folders.len() + 1);
        let gid = folders.get(pick).map(|f| f.gid.as_str());
        if self.s(dev).set_meeting_folder(&[m], gid).is_ok() {
            self.applied += 1;
        }
    }

    fn delete_folder(&mut self, dev: usize, f: u8) {
        let folders = self.s(dev).folders().unwrap();
        if folders.is_empty() {
            return;
        }
        let gid = folders[f as usize % folders.len()].gid.clone();
        if self.s(dev).delete_folder(&gid).is_ok() {
            self.applied += 1;
        }
    }

    fn delete_meeting(&mut self, dev: usize, k: u8) {
        let Some(m) = self.pick_meeting(dev, k) else {
            return;
        };
        self.s(dev).delete_meeting(&m).unwrap();
        self.applied += 1;
    }

    fn regenerate(&mut self, dev: usize, k: u8) {
        let Some(m) = self.pick_meeting(dev, k) else {
            return;
        };
        let (b, a) = (self.text("ai note"), self.text("action"));
        let r = self.s(dev).replace_ai_notes(
            &m,
            vec![note(&b, Provenance::Ai)],
            vec![NewActionItem {
                text: a.clone(),
                ..Default::default()
            }],
        );
        r.unwrap();
        let notes = self.s(dev).note_blocks(&m).unwrap();
        let n = notes.iter().find(|n| n.body == b).unwrap().gid.clone();
        let acts = self.s(dev).action_items(&m).unwrap();
        let act = acts.iter().find(|x| x.text == a).unwrap().gid.clone();
        self.note_written(&b, Field::Note, &m, &n);
        self.note_written(&a, Field::Action, &m, &act);
        self.applied += 1;
    }

    fn retention(&mut self, dev: usize, k: u8) {
        let Some(m) = self.pick_meeting(dev, k) else {
            return;
        };
        self.s(dev).delete_audio(&m).unwrap();
        self.retained.insert(m);
        self.applied += 1;
    }

    // ---------------------------------------------------------------- syncs

    /// A session that may be cut after `cut` writes; `twice` runs it again.
    fn sync_step(&self, spoke: usize, cut: Option<u8>, cut_hub: bool, twice: bool) {
        let _ = try_sync(&self.hub, &self.nodes[spoke], cut.map(usize::from), cut_hub);
        if twice {
            let _ = try_sync(&self.hub, &self.nodes[spoke], None, false);
        }
    }

    /// Sessions until a whole round (both spokes) exchanges nothing.
    fn settle(&self) {
        for _ in 0..24 {
            let mut quiet = true;
            for spoke in 1..=2 {
                let (m, t) = sync(&self.hub, &self.nodes[spoke]);
                quiet &= idle(&m) && idle(&t);
            }
            if quiet {
                return;
            }
        }
        panic!("the devices did not settle in 24 rounds");
    }

    // ----------------------------------------------------------- the checks

    /// Decrypted rows keyed by gid, sorted: what a device holds.
    fn canonical(&self, dev: usize) -> BTreeSet<String> {
        let s = self.s(dev);
        let mut out = BTreeSet::new();
        let meetings = s.list_meetings(10_000, 0).unwrap();
        let gids: Vec<String> = meetings.iter().map(|m| m.gid.clone()).collect();
        for m in &meetings {
            out.insert(format!("meeting {} {m:?}", m.gid));
            for x in s.segments(&m.gid).unwrap() {
                out.insert(format!("segment {} {x:?}", x.gid));
            }
            for x in s.speakers(&m.gid).unwrap() {
                out.insert(format!("speaker {} {x:?}", x.gid));
            }
            for x in s.note_blocks(&m.gid).unwrap() {
                out.insert(format!("note {} {x:?}", x.gid));
            }
            for x in s.action_items(&m.gid).unwrap() {
                out.insert(format!("action {} {x:?}", x.gid));
            }
            for x in s.conflict_copies(&m.gid).unwrap() {
                out.insert(format!("copy {} {x:?}", x.gid));
            }
        }
        for f in s.folders().unwrap() {
            out.insert(format!("folder {} {}", f.gid, f.name));
        }
        for t in s.tags().unwrap() {
            out.insert(format!("tag {} {}", t.gid, t.name));
        }
        for (m, tags) in s.meeting_tags(&gids).unwrap() {
            let mut g: Vec<String> = tags.into_iter().map(|t| t.gid).collect();
            g.sort();
            out.insert(format!("tagged {m} {g:?}"));
        }
        for p in s
            .people_overview()
            .unwrap()
            .into_iter()
            .filter(|p| !p.is_me)
        {
            out.insert(format!("person {} {}", p.gid, p.name));
        }
        for t in s.tombstones_since(0).unwrap() {
            out.insert(format!("tombstone {} {}", t.gid, t.kind));
        }
        out
    }

    /// The gids a device holds alive.
    fn live_gids(&self, dev: usize) -> BTreeSet<String> {
        self.canonical(dev)
            .into_iter()
            .filter(|l| !l.starts_with("tombstone") && !l.starts_with("tagged"))
            .filter_map(|l| l.split(' ').nth(1).map(str::to_string))
            .collect()
    }

    fn assert_converged(&self) {
        let states: Vec<BTreeSet<String>> = (0..3).map(|d| self.canonical(d)).collect();
        for d in 1..3 {
            let only_hub: Vec<_> = states[0].difference(&states[d]).collect();
            let only_dev: Vec<_> = states[d].difference(&states[0]).collect();
            assert!(
                only_hub.is_empty() && only_dev.is_empty(),
                "device {d} differs from the hub.\nonly on the hub: {only_hub:#?}\nonly on {d}: {only_dev:#?}"
            );
        }
        // No tombstoned gid is alive anywhere.
        let dead: BTreeSet<String> = self
            .s(0)
            .tombstones_since(0)
            .unwrap()
            .into_iter()
            .map(|t| t.gid)
            .collect();
        for d in 0..3 {
            let alive = self.live_gids(d);
            let zombies: Vec<_> = dead.intersection(&alive).collect();
            assert!(
                zombies.is_empty(),
                "tombstoned but alive on {d}: {zombies:?}"
            );
        }
        // Audio: a cut on any device removed it everywhere; an uncut track
        // is still on the device that recorded it.
        for (m, dev) in &self.audio_on {
            if self.s(0).is_tombstoned(m).unwrap() {
                continue;
            }
            if self.retained.contains(m) {
                for d in 0..3 {
                    assert!(
                        self.s(d).tracks(m).map(|t| t.is_empty()).unwrap_or(true),
                        "audio of {m} survives a retention cut on {d}"
                    );
                }
            } else {
                assert!(
                    !self.s(*dev).tracks(m).unwrap().is_empty(),
                    "the recording device lost its audio"
                );
            }
        }
        self.assert_free_text();
    }

    /// Every unreplaced text is the current value, a conflict copy, or its
    /// row (or meeting) is under a tombstone.
    fn assert_free_text(&self) {
        let hub = self.s(0);
        for (text, w) in &self.written {
            if w.overwritten
                || hub.is_tombstoned(&w.gid).unwrap()
                || hub.is_tombstoned(&w.meeting).unwrap()
            {
                continue;
            }
            let current = match w.field {
                Field::Title => vec![hub.get_meeting(&w.meeting).unwrap().title],
                Field::Segment => vec![segment_text(hub, &w.meeting, &w.gid)],
                Field::Note => hub
                    .note_blocks(&w.meeting)
                    .unwrap()
                    .into_iter()
                    .filter(|n| n.gid == w.gid)
                    .map(|n| n.body)
                    .collect(),
                Field::Action => hub
                    .action_items(&w.meeting)
                    .unwrap()
                    .into_iter()
                    .filter(|a| a.gid == w.gid)
                    .map(|a| a.text)
                    .collect(),
                Field::Speaker => hub
                    .speakers(&w.meeting)
                    .unwrap()
                    .into_iter()
                    .filter(|s| s.gid == w.gid)
                    .filter_map(|s| s.display_name)
                    .collect(),
            };
            let kept = hub
                .conflict_copies(&w.meeting)
                .unwrap()
                .into_iter()
                .any(|c| c.target_gid == w.gid && &c.text == text);
            assert!(
                current.contains(text) || kept,
                "{:?} text {text:?} of {} was lost (current {current:?})",
                w.field,
                w.gid
            );
        }
    }
}

fn segment_text(s: &Store, meeting: &str, gid: &str) -> String {
    s.segments(meeting)
        .unwrap()
        .into_iter()
        .find(|x| x.gid == gid)
        .map(|x| x.text)
        .unwrap_or_default()
}

// ------------------------------------------------------- random operations

#[derive(Debug, Clone)]
enum Step {
    Do(u8, u8, [u8; 3]),
    Sync(u8, Option<u8>, bool, bool),
}

/// Weighted table of operations; `kind` picks a slot.
const OPS: [u8; 24] = [
    0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 6, 7, 8, 8, 9, 10, 11, 12, 13, 14, 15, 15,
];

fn apply(w: &mut World, dev: usize, kind: u8, a: [u8; 3]) {
    let [x, y, z] = a;
    match OPS[kind as usize % OPS.len()] {
        0 => w.create(dev, x % 3, y % 2 == 0, z % 3),
        1 => w.edit_title(dev, x),
        2 => w.edit_note(dev, x, y),
        3 => w.edit_segment(dev, x, y),
        4 => w.rename(dev, x, y, z % 2 == 0),
        5 => w.merge(dev, x, y, z),
        6 => w.folder(dev, x),
        7 => w.tag(dev, x),
        8 => w.tag_meeting(dev, x, y, false),
        9 => w.tag_meeting(dev, x, y, true),
        10 => w.set_folder(dev, x, y),
        11 => w.delete_meeting(dev, x),
        12 => w.regenerate(dev, x),
        13 => w.retention(dev, x),
        14 => w.delete_folder(dev, x),
        _ => w.edit_title(dev, x),
    }
}

fn run(steps: &[Step]) {
    let mut w = World::new();
    for step in steps {
        match step {
            Step::Do(dev, kind, a) => apply(&mut w, usize::from(*dev), *kind, *a),
            Step::Sync(spoke, cut, cut_hub, twice) => {
                w.sync_step(usize::from(*spoke), *cut, *cut_hub, *twice)
            }
        }
    }
    w.settle();
    w.assert_converged();
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        6 => (0u8..3, any::<u8>(), any::<[u8; 3]>()).prop_map(|(d, k, a)| Step::Do(d, k, a)),
        4 => (1u8..3, proptest::option::of(0u8..48), any::<bool>(), any::<bool>())
            .prop_map(|(s, c, h, t)| Step::Sync(s, c, h, t)),
    ]
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: cases(),
        max_shrink_iters: 24,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn three_devices_converge_after_random_operations_and_interrupted_syncs(
        steps in proptest::collection::vec(step(), 4..32)
    ) {
        run(&steps);
    }
}

// ------------------------------------------------------- fixed scenarios

use Step::{Do, Sync as Sy};

/// Operation codes by their slot in [`OPS`].
const CREATE: u8 = 0;
const FOLDER: u8 = 12; // OPS[12] = 6
const TAG: u8 = 13; // OPS[13] = 7

#[test]
fn same_name_folders_and_tags_made_on_two_spokes_fold_into_one() {
    let mut w = World::new();
    apply(&mut w, 1, CREATE, [1, 1, 0]);
    w.sync_step(1, None, false, false);
    w.settle();
    // Each spoke makes "Work" and "alpha" while the other has not seen it.
    for dev in [1, 2] {
        apply(&mut w, dev, FOLDER, [0, 0, 0]);
        apply(&mut w, dev, TAG, [0, 0, 0]);
    }
    // Each files and tags the meeting with its own copy of the name.
    for dev in [1, 2] {
        w.set_folder(dev, 0, 0);
        w.tag_meeting(dev, 0, 0, false);
    }
    assert_ne!(
        w.s(1).folders().unwrap()[0].gid,
        w.s(2).folders().unwrap()[0].gid,
        "two devices make two folders before they meet"
    );
    w.settle();
    w.assert_converged();
    for dev in 0..3 {
        assert_eq!(w.s(dev).folders().unwrap().len(), 1, "device {dev}");
        assert_eq!(w.s(dev).tags().unwrap().len(), 1, "device {dev}");
        let m = &w.meetings(dev)[0];
        let tags = w.s(dev).meeting_tags(std::slice::from_ref(m)).unwrap();
        assert_eq!(tags[m].len(), 1, "one link to the one tag on {dev}");
        assert!(w.s(dev).get_meeting(m).unwrap().folder_gid.is_some());
    }
    // The folded folder is the lower gid on every device.
    let gids: Vec<String> = (0..3)
        .map(|d| w.s(d).folders().unwrap()[0].gid.clone())
        .collect();
    assert!(gids.iter().all(|g| g == &gids[0]));
}

#[test]
fn speaker_merges_in_opposite_directions_make_a_cycle_every_device_reads_alike() {
    let mut w = World::new();
    apply(&mut w, 1, CREATE, [1, 1, 0]);
    w.sync_step(1, None, false, false);
    w.settle();
    let m = w.meetings(1)[0].clone();
    let sp: Vec<String> = w
        .s(1)
        .speakers(&m)
        .unwrap()
        .into_iter()
        .map(|s| s.gid)
        .collect();
    assert!(sp.len() >= 2);
    // Concurrent: spoke 1 folds 0 into 1, spoke 2 folds 1 into 0.
    w.s(1).merge_speakers(&sp[0], &sp[1]).unwrap();
    w.s(2).merge_speakers(&sp[1], &sp[0]).unwrap();
    w.settle();
    w.assert_converged();
    // Whatever the rule picked, all three agree on every speaker's target.
    let view = |d: usize| -> Vec<(String, Option<String>)> {
        w.s(d)
            .speakers(&m)
            .unwrap()
            .into_iter()
            .map(|s| (s.gid, s.merged_into))
            .collect()
    };
    assert_eq!(view(0), view(1));
    assert_eq!(view(0), view(2));
    let into: Vec<_> = view(0).into_iter().filter_map(|(_, t)| t).collect();
    assert!(
        into.len() <= 1,
        "a cycle is broken: at most one of the two still points at the other, got {into:?}"
    );
}

#[test]
fn a_meeting_deleted_on_one_spoke_while_the_other_edits_ends_dead_everywhere() {
    let mut w = World::new();
    apply(&mut w, 1, CREATE, [2, 1, 1]);
    w.sync_step(1, None, false, false);
    w.settle();
    let m = w.meetings(1)[0].clone();
    w.edit_title(2, 0);
    w.edit_segment(2, 0, 1);
    w.edit_note(2, 0, 0);
    w.delete_meeting(1, 0);
    w.settle();
    w.assert_converged();
    for d in 0..3 {
        assert!(w.s(d).get_meeting(&m).is_err(), "device {d} still has it");
    }
    assert!(w.s(0).is_tombstoned(&m).unwrap());
}

#[test]
fn concurrent_regeneration_and_edits_of_the_same_notes_converge() {
    let mut w = World::new();
    apply(&mut w, 1, CREATE, [1, 1, 2]);
    w.sync_step(1, None, false, false);
    w.settle();
    w.regenerate(1, 0);
    w.edit_note(2, 0, 1);
    w.regenerate(0, 0);
    w.settle();
    w.assert_converged();
}

#[test]
fn a_cut_session_resumes_and_the_third_device_catches_up() {
    let mut w = World::new();
    apply(&mut w, 1, CREATE, [2, 0, 1]);
    apply(&mut w, 1, CREATE, [1, 0, 0]);
    // Cut at every length from the handshake to the end of a long session.
    for cut in [0u8, 1, 2, 3, 5, 8, 13, 21, 34] {
        w.sync_step(1, Some(cut), cut % 2 == 0, false);
    }
    w.settle();
    w.assert_converged();
    assert_eq!(w.meetings(2).len(), 2);
}

#[test]
fn the_scripted_walk_is_replayable() {
    // A short fixed walk through the random harness (a regression anchor).
    run(&[
        Do(1, 0, [2, 0, 1]),
        Sy(1, None, false, false),
        Do(2, 3, [0, 0, 0]),
        Do(0, 3, [0, 0, 0]),
        Sy(2, Some(4), true, true),
        Do(1, 18, [0, 1, 0]),
        Do(2, 18, [0, 1, 0]),
        Sy(1, Some(2), false, false),
        Do(2, 11, [0, 0, 1]),
    ]);
}

/// `Store::merge_speakers` accepts a speaker that is merged away already (the
/// app refuses it in `merge_speakers_in`), so one device can write a cycle on
/// its own. The peer then never gets the meeting's lines or speakers, and the
/// session still reports quiescence.
#[test]
fn a_cycle_written_on_one_device_still_reaches_the_hub() {
    let mut w = World::new();
    apply(&mut w, 1, CREATE, [2, 1, 0]);
    let m = w.meetings(1)[0].clone();
    let sp: Vec<String> = w
        .s(1)
        .speakers(&m)
        .unwrap()
        .into_iter()
        .map(|s| s.gid)
        .collect();
    w.s(1).merge_speakers(&sp[0], &sp[1]).unwrap();
    // The store refuses the second merge, so no cycle is written.
    assert!(matches!(
        w.s(1).merge_speakers(&sp[1], &sp[0]),
        Err(ghi_store::StoreError::AlreadyMerged { .. })
    ));
    w.settle();
    assert_eq!(
        w.s(0).segments(&m).unwrap().len(),
        3,
        "the hub has the lines"
    );
    assert_eq!(
        w.s(0).speakers(&m).unwrap().len(),
        2,
        "the hub has the speakers"
    );
}
