// SPDX-License-Identifier: Apache-2.0
//! People (phase 14c): migration 0006, name links, Me, overview counts, merge,
//! "remove name from notes" and orphan clean-up.

mod common;

use ghi_store::db;
use ghi_store::embeddings::EmbeddingChunk;
use ghi_store::migrate::MIGRATIONS;
use ghi_store::search::{HitKind, SearchQuery};
use ghi_store::store::{NewActionItem, NewNoteBlock, NewSegment, NewSpeaker, Provenance, Store};
use ghi_store::voice::{ThirdPartyApproved, VoiceConsent, VoiceExemplar};

fn approved() -> Option<ThirdPartyApproved> {
    Some(ThirdPartyApproved::assert_flag_checked())
}

fn consent() -> VoiceConsent {
    VoiceConsent {
        method: "self_checkbox".into(),
        at_ms: 1_700_000_000_000,
        text_key: "consent.self.body".into(),
        clip: None,
    }
}

fn exemplar(x: f32) -> VoiceExemplar {
    VoiceExemplar {
        vec: vec![x, 1.0 - x, 0.5, 0.25],
        source: None,
    }
}

fn speaker(store: &Store, meeting: &str, label_idx: i64, name: Option<&str>) -> String {
    let gid = store
        .add_speaker(
            meeting,
            NewSpeaker {
                label_idx,
                ..Default::default()
            },
        )
        .unwrap();
    if let Some(n) = name {
        store.rename_speaker(&gid, Some(n)).unwrap();
    }
    gid
}

fn speaker_of(store: &Store, meeting: &str, gid: &str) -> ghi_store::store::Speaker {
    store
        .speakers(meeting)
        .unwrap()
        .into_iter()
        .find(|s| s.gid == gid)
        .unwrap()
}

fn person_count(store: &Store) -> usize {
    store.people_overview().unwrap().len()
}

fn ready_meeting(store: &Store, title: &str) -> String {
    let gid = common::meeting(store, title);
    store
        .add_segments(&gid, vec![common::seg(0, 1000, "Chốt ngân sách")])
        .unwrap();
    store.set_meeting_status(&gid, "ready").unwrap();
    gid
}

#[test]
fn migration_0006_creates_me_and_links_me_speakers() {
    let tmp = tempfile::tempdir().unwrap();
    let keys = common::keys();
    let store = common::open_with(tmp.path(), &keys, &MIGRATIONS[..5]).unwrap();
    let m = common::meeting(&store, "Cũ");
    drop(store);
    let path = tmp.path().join("ghira.db");
    let conn = db::open(&path, &common::db_key(&keys)).unwrap();
    assert_eq!(db::user_version(&conn).unwrap(), 5);
    let mid: i64 = conn
        .query_row("SELECT id FROM meetings WHERE gid = ?1", [&m], |r| r.get(0))
        .unwrap();
    for (gid, idx, me) in [("s-me", 0, 1), ("s-other", 1, 0)] {
        conn.execute(
            "INSERT INTO speakers (gid, meeting_id, label_idx, is_me) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![gid, mid, idx, me],
        )
        .unwrap();
    }
    drop(conn);

    let store = common::open_with(tmp.path(), &keys, MIGRATIONS).unwrap();
    let me = store.me_person().unwrap();
    let speakers = store.speakers(&m).unwrap();
    let mine = speakers.iter().find(|s| s.is_me).unwrap();
    assert_eq!(mine.person_gid.as_deref(), Some(me.as_str()));
    assert_eq!(speakers.iter().find(|s| !s.is_me).unwrap().person_gid, None);
    let people = store.people_overview().unwrap();
    assert_eq!(people.len(), 1);
    assert!(people[0].is_me && people[0].name.is_empty());
    assert_eq!((people[0].meetings, people[0].voice.clone()), (1, None));
    drop(store);
    let conn = db::open(&path, &common::db_key(&keys)).unwrap();
    assert_eq!(db::user_version(&conn).unwrap(), 6);
    for t in ["voice_profiles", "voice_embeddings", "speaker_voices"] {
        let n: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "{t}");
    }
}

#[test]
fn migration_0006_refuses_old_voice_rows_and_rolls_back() {
    let tmp = tempfile::tempdir().unwrap();
    let keys = common::keys();
    drop(common::open_with(tmp.path(), &keys, &MIGRATIONS[..5]).unwrap());
    let path = tmp.path().join("ghira.db");
    let conn = db::open(&path, &common::db_key(&keys)).unwrap();
    conn.execute(
        "INSERT INTO voice_profiles (gid, consent_json, key_wrapped, created_at)
         VALUES ('v', '{}', x'00', 1)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO voice_embeddings (profile_id, lang, vec_ct) VALUES (1, 'vi', x'00')",
        [],
    )
    .unwrap();
    drop(conn);
    let Err(err) = common::open_with(tmp.path(), &keys, MIGRATIONS) else {
        panic!("must refuse");
    };
    assert!(
        matches!(err, ghi_store::StoreError::Migration { version: 6, .. }),
        "{err}"
    );
    let conn = db::open(&path, &common::db_key(&keys)).unwrap();
    assert_eq!(db::user_version(&conn).unwrap(), 5);
}

#[test]
fn rename_links_unlinks_and_matches_case_but_not_accents() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m1 = common::meeting(&store, "Một");
    let m2 = common::meeting(&store, "Hai");
    let a = speaker(&store, &m1, 0, Some("Minh"));
    let b = speaker(&store, &m2, 0, Some("  minh "));
    let c = speaker(&store, &m2, 1, Some("Mính"));
    let pa = speaker_of(&store, &m1, &a).person_gid.unwrap();
    assert_eq!(
        speaker_of(&store, &m2, &b).person_gid.as_deref(),
        Some(pa.as_str())
    );
    let pc = speaker_of(&store, &m2, &c).person_gid.unwrap();
    assert_ne!(pa, pc, "accents count");
    // Me + the two people.
    assert_eq!(person_count(&store), 3);
    // Names are stored as typed (trimmed), the first spelling wins.
    let overview = store.people_overview().unwrap();
    assert!(overview.iter().any(|p| p.gid == pa && p.name == "Minh"));
    // Decomposed input matches the precomposed name.
    let d = speaker(&store, &m1, 1, Some("Mi\u{0301}nh"));
    assert_eq!(
        speaker_of(&store, &m1, &d).person_gid.as_deref(),
        Some(pc.as_str())
    );

    // Renaming moves the link; clearing unlinks and drops a person nobody has.
    store.rename_speaker(&c, Some("Bình")).unwrap();
    store.rename_speaker(&d, Some("Bình")).unwrap();
    assert!(store.find_person_by_name("Mính").unwrap().is_none());
    store.rename_speaker(&d, None).unwrap();
    assert_eq!(speaker_of(&store, &m1, &d).person_gid, None);
    assert!(store.find_person_by_name("Bình").unwrap().is_some());
    store.rename_speaker(&c, None).unwrap();
    assert!(store.find_person_by_name("Bình").unwrap().is_none());
    assert!(store.is_tombstoned(&pc).unwrap());
}

#[test]
fn not_a_person_and_me_are_never_linked_by_name() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let tv = speaker(&store, &m, 0, None);
    store.set_speaker_not_person(&tv, true).unwrap();
    store.rename_speaker(&tv, Some("Tivi")).unwrap();
    assert_eq!(speaker_of(&store, &m, &tv).person_gid, None);

    let me = store
        .add_speaker(
            &m,
            NewSpeaker {
                label_idx: 1,
                is_me: true,
                ..Default::default()
            },
        )
        .unwrap();
    store.rename_speaker(&me, Some("Tôi")).unwrap();
    let mine = speaker_of(&store, &m, &me);
    assert_eq!(
        mine.person_gid.as_deref(),
        Some(store.me_person().unwrap().as_str())
    );
    store.rename_speaker(&me, None).unwrap();
    assert_eq!(
        speaker_of(&store, &m, &me).person_gid.as_deref(),
        Some(store.me_person().unwrap().as_str())
    );
    assert_eq!(person_count(&store), 1, "only Me");
}

#[test]
fn link_named_speakers_backfills_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    // Named without a link, as meetings were before people existed.
    let mut raw = Vec::new();
    for (i, name) in ["Lan", "lan", "Đức"].into_iter().enumerate() {
        let gid = store
            .add_speaker(
                &m,
                NewSpeaker {
                    label_idx: i as i64,
                    display_name: Some(name.into()),
                    ..Default::default()
                },
            )
            .unwrap();
        raw.push(gid);
    }
    let unnamed = speaker(&store, &m, 3, None);
    assert_eq!(person_count(&store), 1);
    assert_eq!(store.link_named_speakers().unwrap(), 3);
    assert_eq!(store.link_named_speakers().unwrap(), 0);
    assert_eq!(person_count(&store), 3, "Me, Lan, Đức");
    let s = store.speakers(&m).unwrap();
    assert_eq!(s[0].person_gid, s[1].person_gid);
    assert!(s[0].person_gid.is_some() && s[2].person_gid.is_some());
    assert_eq!(speaker_of(&store, &m, &unnamed).person_gid, None);
}

#[test]
fn overview_counts_match_hand_computed() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m1 = common::meeting(&store, "Một");
    let m2 = common::meeting(&store, "Hai");
    let m3 = common::meeting(&store, "Ba");
    let an1 = speaker(&store, &m1, 0, Some("An"));
    let an1b = speaker(&store, &m1, 1, Some("An")); // second speaker, same meeting
    let an2 = speaker(&store, &m2, 0, Some("An"));
    let bo = speaker(&store, &m3, 0, Some("Bo"));
    // An's speaker in m1 (the 'b' one) is merged away: not counted again.
    store.merge_speakers(&an1b, &an1).unwrap();
    let new_action = |m: &str, owner: &str, text: &str| {
        store
            .add_action_item(
                m,
                NewActionItem {
                    text: text.into(),
                    owner_speaker_gid: Some(owner.into()),
                    ..Default::default()
                },
            )
            .unwrap()
    };
    let open1 = new_action(&m1, &an1, "việc một");
    let _open2 = new_action(&m2, &an2, "việc hai");
    let done = new_action(&m2, &an2, "việc xong");
    let _bo_open = new_action(&m3, &bo, "việc của Bo");
    store.set_action_done(&done.gid, true).unwrap();
    // An action whose owner was merged into `an1` counts for An once.
    let moved = new_action(&m1, &an1b, "đã gộp");
    let _ = (open1, moved);

    let people = store.people_overview().unwrap();
    let get = |n: &str| people.iter().find(|p| p.name == n).unwrap();
    // `an1b` was merged away; its action followed the merge only if it was
    // there at merge time, so it still points at the merged-away speaker and
    // is not counted. Open: open1, _open2.
    assert_eq!((get("An").meetings, get("An").open_actions), (2, 2));
    assert_eq!((get("Bo").meetings, get("Bo").open_actions), (1, 1));
    assert!(people[0].is_me && people[0].meetings == 0);
    let started = |m: &str| store.get_meeting(m).unwrap().started_at;
    assert_eq!(get("An").last_met_ms, Some(started(&m1).max(started(&m2))));

    let meetings = store.person_meetings(&get("An").gid, 10).unwrap();
    assert_eq!(meetings.len(), 2);
    assert!(meetings.iter().any(|m| m.title == "Một"));
    assert_eq!(store.person_meetings(&get("An").gid, 1).unwrap().len(), 1);
    let actions = store.person_open_actions(&get("An").gid).unwrap();
    let mut texts: Vec<_> = actions.iter().map(|a| a.text.as_str()).collect();
    texts.sort();
    assert_eq!(texts, ["việc hai", "việc một"]);
    assert!(actions.iter().all(|a| !a.meeting_title.is_empty()));
}

#[test]
fn set_speaker_me_moves_me_within_the_meeting() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let a = speaker(&store, &m, 0, Some("An"));
    let b = speaker(&store, &m, 1, None);
    store.set_speaker_me(&a).unwrap();
    let me = store.me_person().unwrap();
    let sa = speaker_of(&store, &m, &a);
    assert!(sa.is_me);
    assert_eq!(sa.person_gid.as_deref(), Some(me.as_str()));
    assert_eq!(sa.display_name.as_deref(), Some("An"), "the name stays");
    assert!(
        store.find_person_by_name("An").unwrap().is_none(),
        "orphan person gone"
    );
    store.set_speaker_me(&b).unwrap();
    assert!(!speaker_of(&store, &m, &a).is_me);
    assert_eq!(speaker_of(&store, &m, &a).person_gid, None);
    assert!(speaker_of(&store, &m, &b).is_me);
    // A merged-away speaker can't become Me.
    let c = speaker(&store, &m, 2, None);
    store.merge_speakers(&c, &b).unwrap();
    assert!(store.set_speaker_me(&c).is_err());
    // Merging Me into another speaker carries Me.
    let d = speaker(&store, &m, 3, Some("Dũng"));
    store.merge_speakers(&b, &d).unwrap();
    let sd = speaker_of(&store, &m, &d);
    assert!(sd.is_me && sd.person_gid.as_deref() == Some(me.as_str()));
    assert!(store.find_person_by_name("Dũng").unwrap().is_none());
}

#[test]
fn suggestions_are_stored_shown_and_cleared_by_naming() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let other = speaker(&store, &m, 0, Some("Hạnh"));
    let person = speaker_of(&store, &m, &other).person_gid.unwrap();
    let s = speaker(&store, &m, 1, None);
    store
        .set_speaker_suggestion(&s, Some((&person, 0.62)))
        .unwrap();
    let sug = speaker_of(&store, &m, &s).suggestion.unwrap();
    assert_eq!(
        (sug.person_gid.as_str(), sug.person_name.as_str()),
        (person.as_str(), "Hạnh")
    );
    assert!((sug.score - 0.62).abs() < 1e-6);
    store.rename_speaker(&s, Some("Khác")).unwrap();
    assert_eq!(speaker_of(&store, &m, &s).suggestion, None);
    store
        .set_speaker_suggestion(&s, Some((&person, 0.5)))
        .unwrap();
    store.set_speaker_suggestion(&s, None).unwrap();
    assert_eq!(speaker_of(&store, &m, &s).suggestion, None);
    assert!(store.set_speaker_suggestion("nope", None).is_err());
}

#[test]
fn merge_persons_moves_speakers_and_reseals_names() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m1 = common::meeting(&store, "Một");
    let m2 = common::meeting(&store, "Hai");
    let a = speaker(&store, &m1, 0, Some("Minh"));
    let b = speaker(&store, &m2, 0, Some("Minh B"));
    let sug = speaker(&store, &m2, 1, None);
    let pa = speaker_of(&store, &m1, &a).person_gid.unwrap();
    let pb = speaker_of(&store, &m2, &b).person_gid.unwrap();
    store
        .set_speaker_suggestion(&sug, Some((&pb, 0.6)))
        .unwrap();

    let affected = store.merge_persons(&pb, &pa).unwrap();
    assert_eq!(affected, vec![m2.clone()]);
    let sb = speaker_of(&store, &m2, &b);
    assert_eq!(sb.person_gid.as_deref(), Some(pa.as_str()));
    assert_eq!(sb.display_name.as_deref(), Some("Minh"));
    assert_eq!(
        speaker_of(&store, &m2, &sug).suggestion.unwrap().person_gid,
        pa
    );
    assert!(store.is_tombstoned(&pb).unwrap());
    assert!(store.find_person_by_name("Minh B").unwrap().is_none());
    assert_eq!(store.person(&pa).unwrap().meetings, 2);

    // Me can't be merged, in either direction; nor a person into itself.
    let me = store.me_person().unwrap();
    assert!(store.merge_persons(&me, &pa).is_err());
    assert!(store.merge_persons(&pa, &me).is_err());
    assert!(store.merge_persons(&pa, &pa).is_err());
}

#[test]
fn merge_persons_merges_voice_profiles_and_shreds_the_source() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let a = speaker(&store, &m, 0, Some("An"));
    let b = speaker(&store, &m, 1, Some("Bo"));
    let pa = speaker_of(&store, &m, &a).person_gid.unwrap();
    let pb = speaker_of(&store, &m, &b).person_gid.unwrap();
    let put = |p: &str, lang: &str, xs: &[f32]| {
        store
            .put_voice_profile(
                p,
                &consent(),
                None,
                "model",
                vec![(lang.into(), xs.iter().map(|x| exemplar(*x)).collect())],
                approved(),
            )
            .unwrap()
    };
    let ga = put(&pa, "vi", &[0.1, 0.2]);
    let gb = put(&pb, "vi", &[0.8, 0.9]);
    put_en(&store, &gb);
    store.merge_persons(&pb, &pa).unwrap();

    let p = store.voice_profile(&pa).unwrap().unwrap();
    assert_eq!(p.gid, ga, "the target's profile survives");
    let vi = p.sets.iter().find(|s| s.lang == "vi").unwrap();
    assert_eq!(vi.exemplars.len(), 4);
    // Centroid = normalised mean of the four.
    let mean: Vec<f32> = (0..4)
        .map(|i| vi.exemplars.iter().map(|e| e.vec[i]).sum::<f32>() / 4.0)
        .collect();
    let norm = mean.iter().map(|x| x * x).sum::<f32>().sqrt();
    for (c, m) in vi.centroid.iter().zip(&mean) {
        assert!((c - m / norm).abs() < 1e-5);
    }
    assert!(
        p.sets.iter().any(|s| s.lang == "en"),
        "languages are unioned"
    );
    assert!(store.is_tombstoned(&gb).unwrap());
    let approval = ThirdPartyApproved::assert_flag_checked();
    assert_eq!(
        store
            .third_party_voice_profiles("model", approval)
            .unwrap()
            .len(),
        1
    );

    // Only the source has a profile: it moves to the target.
    let c = speaker(&store, &m, 2, Some("Chi"));
    let d = speaker(&store, &m, 3, Some("Dao"));
    let pc = speaker_of(&store, &m, &c).person_gid.unwrap();
    let pd = speaker_of(&store, &m, &d).person_gid.unwrap();
    let gd = put(&pd, "vi", &[0.3]);
    store.merge_persons(&pd, &pc).unwrap();
    assert_eq!(store.voice_profile(&pc).unwrap().unwrap().gid, gd);
}

fn put_en(store: &Store, profile: &str) {
    store
        .add_voice_exemplars(profile, "model", "en", vec![exemplar(0.4)], approved())
        .unwrap();
}

#[test]
fn remove_person_name_rewrites_ai_notes_and_keeps_the_voice_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = ready_meeting(&store, "Họp");
    let other = ready_meeting(&store, "Khác");
    let minh = speaker(&store, &m, 1, Some("Quang"));
    let pid = speaker_of(&store, &m, &minh).person_gid.unwrap();
    let in_other = speaker(&store, &other, 0, Some("Quang"));
    let _ = in_other;
    let bystander = speaker(&store, &m, 0, Some("Lan"));
    let _ = bystander;
    store
        .put_voice_profile(
            &pid,
            &consent(),
            None,
            "model",
            vec![("vi".into(), vec![exemplar(0.2)])],
            approved(),
        )
        .unwrap();

    let note = |prov, body: &str, pinned| {
        store
            .add_note_block(
                &m,
                NewNoteBlock {
                    kind: "bullet".into(),
                    provenance: prov,
                    body: body.into(),
                    anchors: vec![],
                    pinned,
                },
            )
            .unwrap()
    };
    let ai = note(
        Provenance::Ai,
        "Quang chốt ngân sách; Quangs không đổi.",
        false,
    );
    let edited = note(Provenance::AiEdited, "Giao cho Quang", false);
    let user = note(Provenance::User, "Quang tự viết", false);
    let act = store
        .add_action_item(
            &m,
            NewActionItem {
                text: "Quang gửi báo cáo".into(),
                provenance: Provenance::Ai,
                ..Default::default()
            },
        )
        .unwrap();
    let user_act = store
        .add_action_item(
            &m,
            NewActionItem {
                text: "Quang gọi khách".into(),
                provenance: Provenance::User,
                ..Default::default()
            },
        )
        .unwrap();
    let v = store.get_meeting(&m).unwrap().transcript_version;
    store
        .put_embeddings(
            &m,
            "e5",
            v,
            store.index_gen(&m).unwrap(),
            vec![EmbeddingChunk {
                chunk: 0,
                t0_ms: 0,
                t1_ms: 1000,
                vec: vec![1.0, 0.0],
            }],
        )
        .unwrap();
    assert!(
        store
            .meetings_needing_embeddings("e5", 10)
            .unwrap()
            .contains(&other)
    );
    assert!(
        !store
            .meetings_needing_embeddings("e5", 10)
            .unwrap()
            .contains(&m)
    );
    let q = |t: &str| {
        store
            .search(&SearchQuery::new(t))
            .unwrap()
            .into_iter()
            .filter(|h| h.kind == HitKind::Note)
            .count()
    };
    assert_eq!(q("quang"), 3);

    let affected = store.remove_person_name(&pid).unwrap();
    assert_eq!(affected, vec![m.clone(), other.clone()]);

    // Speakers: names and link gone, in both meetings.
    let s = speaker_of(&store, &m, &minh);
    assert_eq!((s.display_name, s.person_gid), (None, None));
    // AI blocks rewritten with the meeting's label, provenance unchanged.
    let blocks = store.note_blocks(&m).unwrap();
    let get = |gid: &str| blocks.iter().find(|b| b.gid == gid).unwrap();
    assert_eq!(
        get(&ai.gid).body,
        "Speaker 2 chốt ngân sách; Quangs không đổi."
    );
    assert_eq!(get(&ai.gid).provenance, Provenance::Ai);
    assert_eq!(get(&edited.gid).body, "Giao cho Speaker 2");
    assert_eq!(get(&edited.gid).provenance, Provenance::AiEdited);
    assert_eq!(
        get(&user.gid).body,
        "Quang tự viết",
        "user blocks untouched"
    );
    let actions = store.action_items(&m).unwrap();
    let a = |gid: &str| actions.iter().find(|a| a.gid == gid).unwrap();
    assert_eq!(a(&act.gid).text, "Speaker 2 gửi báo cáo");
    assert_eq!(a(&act.gid).provenance, Provenance::Ai);
    assert_eq!(a(&user_act.gid).text, "Quang gọi khách");
    // Notes FTS: only the user block and the literal "Quangs" still match.
    assert_eq!(q("quang"), 2);
    // ... and the index itself was compacted: no "quang" token of an AI block.
    store.checkpoint().unwrap();
    // Embeddings dropped, so both meetings are queued for indexing again.
    assert!(
        store
            .meetings_needing_embeddings("e5", 10)
            .unwrap()
            .contains(&m)
    );
    assert!(store.embeddings(&m, "e5").unwrap().is_empty());
    // The voice profile is intact, so the person row stays (name and all).
    assert!(store.voice_profile(&pid).unwrap().is_some());
    assert_eq!(store.person(&pid).unwrap().meetings, 0);
    // Me has no name to remove.
    assert!(
        store
            .remove_person_name(&store.me_person().unwrap())
            .is_err()
    );
}

#[test]
fn remove_person_name_drops_a_person_without_a_voice_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let s = speaker(&store, &m, 0, Some("An"));
    let p = speaker_of(&store, &m, &s).person_gid.unwrap();
    assert_eq!(store.remove_person_name(&p).unwrap(), vec![m.clone()]);
    assert!(store.find_person_by_name("An").unwrap().is_none());
    assert!(store.is_tombstoned(&p).unwrap());
}

#[test]
fn deleting_a_meeting_removes_orphan_persons_but_keeps_those_with_a_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m1 = common::meeting(&store, "Một");
    let m2 = common::meeting(&store, "Hai");
    let only = speaker(&store, &m1, 0, Some("Chỉ một"));
    let both1 = speaker(&store, &m1, 1, Some("Cả hai"));
    let _both2 = speaker(&store, &m2, 0, Some("Cả hai"));
    let voiced = speaker(&store, &m1, 2, Some("Có giọng"));
    let p_only = speaker_of(&store, &m1, &only).person_gid.unwrap();
    let p_both = speaker_of(&store, &m1, &both1).person_gid.unwrap();
    let p_voiced = speaker_of(&store, &m1, &voiced).person_gid.unwrap();
    store
        .put_voice_profile(
            &p_voiced,
            &consent(),
            None,
            "model",
            vec![("vi".into(), vec![exemplar(0.5)])],
            approved(),
        )
        .unwrap();
    store.delete_meeting(&m1).unwrap();
    assert!(store.find_person_by_name("Chỉ một").unwrap().is_none());
    assert!(store.is_tombstoned(&p_only).unwrap());
    assert!(store.find_person_by_name("Cả hai").unwrap().is_some());
    assert!(!store.is_tombstoned(&p_both).unwrap());
    assert!(store.find_person_by_name("Có giọng").unwrap().is_some());
    assert_eq!(store.person(&p_both).unwrap().meetings, 1);
    // Me is never removed.
    store.delete_meeting(&m2).unwrap();
    assert_eq!(
        store.people_overview().unwrap().len(),
        2,
        "Me and the voiced person"
    );
}

#[test]
fn me_speaker_survives_with_a_transcript_segment() {
    // add_speaker(is_me) + segments: Me counts one meeting.
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let me = store
        .add_speaker(
            &m,
            NewSpeaker {
                is_me: true,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .add_segment(
            &m,
            NewSegment {
                speaker_gid: Some(me),
                t0_ms: 0,
                t1_ms: 10,
                text: "Chào".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(store.people_overview().unwrap()[0].meetings, 1);
}

// ------------------------------------------------------- review follow-ups

fn raw_conn(dir: &std::path::Path, keys: &common::Keys) -> rusqlite::Connection {
    db::open(&dir.join("ghira.db"), &common::db_key(keys)).unwrap()
}

#[test]
fn migration_0006_from_a_v5_database_with_people_data() {
    let tmp = tempfile::tempdir().unwrap();
    let keys = common::keys();
    let store = common::open_with(tmp.path(), &keys, &MIGRATIONS[..5]).unwrap();
    let m = common::meeting(&store, "Cũ");
    drop(store);
    let conn = raw_conn(tmp.path(), &keys);
    let mid: i64 = conn
        .query_row("SELECT id FROM meetings WHERE gid = ?1", [&m], |r| r.get(0))
        .unwrap();
    // Persons as only a test could have made them in v5, two of which are
    // the same name by key, plus speakers and action items pointing at them.
    for (id, name) in [(1, "Minh"), (2, "minh "), (3, "An")] {
        conn.execute(
            "INSERT INTO persons (id, gid, name) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, format!("p{id}"), name],
        )
        .unwrap();
    }
    for (i, person) in [(0, 1), (1, 2), (2, 3)] {
        conn.execute(
            "INSERT INTO speakers (id, gid, meeting_id, label_idx, person_id)
             VALUES (?1, ?2, ?3, ?1, ?4)",
            rusqlite::params![i + 1, format!("s{i}"), mid, person],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO action_items (gid, meeting_id, text_ct, owner_speaker_id)
             VALUES (?1, ?2, x'00', ?3)",
            rusqlite::params![format!("a{i}"), mid, i + 1],
        )
        .unwrap();
    }
    drop(conn);

    let store = common::open_with(tmp.path(), &keys, MIGRATIONS).unwrap();
    let people = store.people_overview().unwrap();
    assert_eq!(people.len(), 3, "Me, Minh (merged), An");
    let minh = people.iter().find(|p| p.name == "Minh").unwrap();
    assert_eq!((minh.meetings, minh.open_actions), (1, 2));
    assert!(store.is_tombstoned("p2").unwrap());
    assert_eq!(
        store.find_person_by_name(" MINH").unwrap().as_deref(),
        Some("p1")
    );
    drop(store);
    let conn = raw_conn(tmp.path(), &keys);
    // The uniqueness is enforced from now on, and the indexes exist.
    assert!(
        conn.execute(
            "INSERT INTO persons (gid, name, name_key) VALUES ('x', 'MINH', 'minh')",
            []
        )
        .is_err()
    );
    for ix in [
        "persons_name_key",
        "action_items_owner",
        "speakers_suggest",
        "voice_profiles_person",
    ] {
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = ?1",
                [ix],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "{ix}");
    }
}

#[test]
fn migration_0006_refuses_voice_profiles_alone_with_a_clear_message() {
    let tmp = tempfile::tempdir().unwrap();
    let keys = common::keys();
    drop(common::open_with(tmp.path(), &keys, &MIGRATIONS[..5]).unwrap());
    let conn = raw_conn(tmp.path(), &keys);
    conn.execute(
        "INSERT INTO voice_profiles (gid, consent_json, key_wrapped, created_at)
         VALUES ('v', '{}', x'00', 1)",
        [],
    )
    .unwrap();
    drop(conn);
    let Err(err) = common::open_with(tmp.path(), &keys, MIGRATIONS) else {
        panic!("must refuse");
    };
    let text = err.to_string();
    assert!(
        text.contains("voice_profiles") && text.contains("refusing"),
        "{text}"
    );
    assert_eq!(
        db::user_version(&raw_conn(tmp.path(), &keys)).unwrap(),
        5,
        "rolled back"
    );
}

#[test]
fn a_case_only_rename_respells_the_person() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let s = speaker(&store, &m, 0, Some("minh"));
    let p = speaker_of(&store, &m, &s).person_gid.unwrap();
    assert_eq!(store.person(&p).unwrap().name, "minh");
    store.rename_speaker(&s, Some("Minh")).unwrap();
    assert_eq!(store.person(&p).unwrap().name, "Minh");
    assert_eq!(
        speaker_of(&store, &m, &s).person_gid.as_deref(),
        Some(p.as_str())
    );
    assert_eq!(person_count(&store), 2);
}

#[test]
fn not_a_person_unlinks_clears_the_suggestion_and_relinks_by_name() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let tv = speaker(&store, &m, 0, Some("Tivi"));
    let p = speaker_of(&store, &m, &tv).person_gid.unwrap();
    let other = speaker(&store, &m, 1, Some("Lan"));
    let lan = speaker_of(&store, &m, &other).person_gid.unwrap();
    store
        .set_speaker_suggestion(&tv, Some((&lan, 0.6)))
        .unwrap();

    store.set_speaker_not_person(&tv, true).unwrap();
    let s = speaker_of(&store, &m, &tv);
    assert!(s.not_person);
    assert_eq!((s.person_gid, s.suggestion), (None, None));
    assert_eq!(s.display_name.as_deref(), Some("Tivi"), "the name stays");
    assert!(
        store.is_tombstoned(&p).unwrap(),
        "nothing else had that person"
    );
    assert!(store.find_person_by_name("Tivi").unwrap().is_none());

    store.set_speaker_not_person(&tv, false).unwrap();
    let again = speaker_of(&store, &m, &tv).person_gid.unwrap();
    assert_eq!(store.person(&again).unwrap().name, "Tivi");
    // A person other speakers still have stays.
    store.set_speaker_not_person(&other, true).unwrap();
    assert!(store.find_person_by_name("Lan").unwrap().is_none());
    let m2 = common::meeting(&store, "M2");
    let lan2 = speaker(&store, &m2, 0, Some("Hà"));
    let lan3 = speaker(&store, &m, 2, Some("Hà"));
    store.set_speaker_not_person(&lan3, true).unwrap();
    assert!(speaker_of(&store, &m2, &lan2).person_gid.is_some());
    assert!(store.find_person_by_name("Hà").unwrap().is_some());
}

#[test]
fn merging_speakers_unlinks_the_merged_one_and_collects_the_orphan() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let m2 = common::meeting(&store, "M2");
    let a = speaker(&store, &m, 0, Some("An"));
    let b = speaker(&store, &m, 1, None);
    let pa = speaker_of(&store, &m, &a).person_gid.unwrap();
    store.merge_speakers(&a, &b).unwrap();
    let sa = speaker_of(&store, &m, &a);
    assert_eq!(sa.person_gid, None, "the merged-away speaker is unlinked");
    assert!(store.is_tombstoned(&pa).unwrap(), "and An had nothing else");

    // With another meeting holding An, the person stays.
    let c = speaker(&store, &m, 2, Some("Bo"));
    let d = speaker(&store, &m, 3, None);
    let _bo2 = speaker(&store, &m2, 0, Some("Bo"));
    let pb = speaker_of(&store, &m, &c).person_gid.unwrap();
    store.merge_speakers(&c, &d).unwrap();
    assert!(!store.is_tombstoned(&pb).unwrap());
    assert_eq!(store.person(&pb).unwrap().meetings, 1);
}

#[test]
fn gc_and_remove_name_ignore_speakers_merged_away_in_older_data() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let a = speaker(&store, &m, 0, None);
    let b = speaker(&store, &m, 1, None);
    store.merge_speakers(&a, &b).unwrap();
    let p2 = store.add_person("Việt", 0).unwrap();
    drop(store);
    // Data from before merging unlinked the person: the merged-away speaker
    // is still linked.
    raw_conn(tmp.path(), &keys)
        .execute(
            "UPDATE speakers SET person_id = (SELECT id FROM persons WHERE gid = ?1) WHERE gid = ?2",
            [&p2, &a],
        )
        .unwrap();
    let store = common::reopen(tmp.path(), &keys);
    // `a` now points at Việt. Link the live speaker to Việt and unlink it:
    // the merged-away speaker doesn't keep the person alive.
    store.set_speaker_person(&b, Some(&p2)).unwrap();
    store.set_speaker_person(&b, None).unwrap();
    assert!(store.is_tombstoned(&p2).unwrap());

    // "Remove name" on a person both are linked to uses the live speaker's number.
    let p3 = store.add_person("Quang", 0).unwrap();
    store.set_speaker_person(&b, Some(&p3)).unwrap();
    drop(store);
    raw_conn(tmp.path(), &keys)
        .execute(
            "UPDATE speakers SET person_id = (SELECT id FROM persons WHERE gid = ?1) WHERE gid = ?2",
            [&p3, &a],
        )
        .unwrap();
    let store = common::reopen(tmp.path(), &keys);
    store
        .add_note_block(
            &m,
            NewNoteBlock {
                kind: "bullet".into(),
                provenance: Provenance::Ai,
                body: "Quang chốt".into(),
                anchors: vec![],
                pinned: false,
            },
        )
        .unwrap();
    assert_eq!(store.remove_person_name(&p3).unwrap(), vec![m.clone()]);
    assert_eq!(store.note_blocks(&m).unwrap()[0].body, "Speaker 2 chốt");
    assert!(
        store
            .speakers(&m)
            .unwrap()
            .iter()
            .all(|s| s.person_gid.is_none())
    );
}

#[test]
fn set_speaker_person_and_add_person_follow_the_link_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let a = store.add_person("  Bình ", 1).unwrap();
    assert_eq!(
        store.add_person("bình", 2).unwrap(),
        a,
        "deduped by name key"
    );
    assert!(store.add_person("   ", 0).is_err());
    let s = speaker(&store, &m, 0, None);
    store.set_speaker_person(&s, Some(&a)).unwrap();
    assert_eq!(
        speaker_of(&store, &m, &s).person_gid.as_deref(),
        Some(a.as_str())
    );
    // Me can't be a target, a Me speaker can't be re-linked, nor a not-a-person one.
    let me = store.me_person().unwrap();
    assert!(store.set_speaker_person(&s, Some(&me)).is_err());
    let mine = store
        .add_speaker(
            &m,
            NewSpeaker {
                label_idx: 1,
                is_me: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(store.set_speaker_person(&mine, Some(&a)).is_err());
    assert!(store.set_speaker_person(&mine, None).is_err());
    let tv = speaker(&store, &m, 2, None);
    store.set_speaker_not_person(&tv, true).unwrap();
    assert!(store.set_speaker_person(&tv, Some(&a)).is_err());
    // Unlinking collects the person nothing else uses.
    store.set_speaker_person(&s, None).unwrap();
    assert!(store.is_tombstoned(&a).unwrap());
}

#[test]
fn a_person_with_a_live_voice_profile_cannot_be_deleted_by_sql() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let s = speaker(&store, &m, 0, Some("An"));
    let p = speaker_of(&store, &m, &s).person_gid.unwrap();
    let g = store
        .put_voice_profile(
            &p,
            &consent(),
            None,
            "model",
            vec![("vi".into(), vec![exemplar(0.1)])],
            approved(),
        )
        .unwrap();
    let conn = raw_conn(tmp.path(), &keys);
    let err = conn
        .execute("DELETE FROM persons WHERE gid = ?1", [&p])
        .unwrap_err();
    assert!(err.to_string().contains("voice profile"), "{err}");
    drop(conn);
    // Through the store: the profile goes first, then the person may.
    store.delete_voice_profile(&g).unwrap();
    assert!(store.voice_profile(&p).unwrap().is_none());
    let conn = raw_conn(tmp.path(), &keys);
    assert_eq!(
        conn.execute("DELETE FROM persons WHERE gid = ?1", [&p])
            .unwrap(),
        1
    );
}

#[test]
fn remove_name_matches_whole_words_with_accents_and_nfd_text() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = ready_meeting(&store, "M");
    let _first = speaker(&store, &m, 0, None);
    let anh = speaker(&store, &m, 1, Some("Ánh"));
    let ha = speaker(&store, &m, 2, Some("Hà"));
    let p_anh = speaker_of(&store, &m, &anh).person_gid.unwrap();
    let p_ha = speaker_of(&store, &m, &ha).person_gid.unwrap();
    let note = |body: &str| {
        store
            .add_note_block(
                &m,
                NewNoteBlock {
                    kind: "bullet".into(),
                    provenance: Provenance::Ai,
                    body: body.into(),
                    anchors: vec![],
                    pinned: false,
                },
            )
            .unwrap()
            .gid
    };
    let n1 = note("Ánh nhắc Anh Minh; Hà ở Hàng Bài, Hà Nội.");
    // The same text decomposed (NFD), as a model might emit it.
    let n2 = note("A\u{0301}nh va Ha\u{0300} gap nhau");
    store.remove_person_name(&p_anh).unwrap();
    store.remove_person_name(&p_ha).unwrap();
    let blocks = store.note_blocks(&m).unwrap();
    let body = |g: &str| blocks.iter().find(|b| b.gid == g).unwrap().body.clone();
    assert_eq!(
        body(&n1),
        "Speaker 2 nhắc Anh Minh; Speaker 3 ở Hàng Bài, Speaker 3 Nội."
    );
    assert_eq!(body(&n2), "Speaker 2 va Speaker 3 gap nhau");
    drop(store);
    // After the compaction the notes index no longer holds the names.
    let conn = raw_conn(tmp.path(), &keys);
    let hits = |q: &str| -> i64 {
        conn.query_row(
            "SELECT count(*) FROM notes_fts WHERE notes_fts MATCH ?1",
            [q],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(hits("anh"), 1, "only the literal 'Anh' of the first block");
    assert_eq!(hits("speaker"), 2);
    assert_eq!(hits("nhau"), 1);
}

#[test]
fn automatic_writes_are_checked_again_and_skip_what_the_user_changed() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = ready_meeting(&store, "m");
    let hoa = store.add_person("Hoa", 1).unwrap();
    let (a, b, c) = (
        speaker(&store, &m, 0, None),
        speaker(&store, &m, 1, Some("Minh")),
        speaker(&store, &m, 2, None),
    );
    // Named by the user: neither a name, a suggestion nor Me lands.
    assert!(!store.rename_speaker_if_unnamed(&b, "Hoa").unwrap());
    assert!(
        !store
            .set_speaker_suggestion_if_unnamed(&b, Some((&hoa, 0.6)))
            .unwrap()
    );
    assert!(!store.set_speaker_me_if_unclaimed(&b).unwrap());
    let sp = speaker_of(&store, &m, &b);
    assert_eq!(sp.display_name.as_deref(), Some("Minh"));
    assert!(sp.suggestion.is_none() && !sp.is_me);
    // Unnamed: they apply.
    assert!(
        store
            .set_speaker_suggestion_if_unnamed(&a, Some((&hoa, 0.6)))
            .unwrap()
    );
    assert!(store.rename_speaker_if_unnamed(&a, "Hoa").unwrap());
    let sp = speaker_of(&store, &m, &a);
    assert_eq!(sp.display_name.as_deref(), Some("Hoa"));
    assert_eq!(sp.person_gid.as_deref(), Some(hoa.as_str()));
    assert!(sp.suggestion.is_none(), "naming settles the suggestion");
    // Me: once claimed, a second automatic claim is refused; a merged or
    // not-a-person speaker is refused.
    assert!(store.set_speaker_me_if_unclaimed(&c).unwrap());
    let d = speaker(&store, &m, 3, None);
    assert!(!store.set_speaker_me_if_unclaimed(&d).unwrap());
    store.set_speaker_not_person(&d, true).unwrap();
    assert!(!store.rename_speaker_if_unnamed(&d, "Lan").unwrap());
    store.merge_speakers(&d, &a).unwrap();
    assert!(
        !store
            .set_speaker_suggestion_if_unnamed(&d, Some((&hoa, 0.6)))
            .unwrap()
    );
    // The user can still move Me explicitly.
    store.set_speaker_me(&a).unwrap();
    assert!(speaker_of(&store, &m, &a).is_me && !speaker_of(&store, &m, &c).is_me);
}

#[test]
fn moving_me_drops_the_exemplars_the_meeting_gave() {
    use ghi_store::voice::ExemplarSource;
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let (m, other) = (ready_meeting(&store, "m"), ready_meeting(&store, "o"));
    let (a, b) = (speaker(&store, &m, 0, None), speaker(&store, &m, 1, None));
    let me = store.me_person().unwrap();
    let from = |g: &str| VoiceExemplar {
        vec: vec![0.5, 0.5, 0.5, 0.5],
        source: Some(ExemplarSource {
            meeting_gid: g.into(),
            t0_ms: 0,
            t1_ms: 1,
        }),
    };
    let profile = store
        .put_voice_profile(
            &me,
            &consent(),
            None,
            "m1",
            vec![("any".into(), vec![exemplar(0.5)])],
            None,
        )
        .unwrap();
    store.set_speaker_me(&a).unwrap();
    store
        .add_voice_exemplars(&profile, "m1", "vi", vec![from(&m), from(&other)], None)
        .unwrap();
    let n = |store: &Store| {
        store
            .me_voice_profile("m1")
            .unwrap()
            .unwrap()
            .sets
            .iter()
            .map(|s| s.exemplars.len())
            .sum::<usize>()
    };
    assert_eq!(n(&store), 3);
    // Re-marking the same speaker moves nothing.
    store.set_speaker_me(&a).unwrap();
    assert_eq!(n(&store), 3);
    // Me is somebody else: this meeting's exemplar goes, the other's stays.
    store.set_speaker_me(&b).unwrap();
    assert_eq!(n(&store), 2);
    // The explicit API, and a set left empty goes with them.
    assert_eq!(
        store
            .drop_voice_exemplars_from(&profile, &other, None)
            .unwrap(),
        1
    );
    let p = store.me_voice_profile("m1").unwrap().unwrap();
    assert_eq!(p.sets.len(), 1, "the vi set is gone");
    assert_eq!(
        store
            .drop_voice_exemplars_from(&profile, &other, None)
            .unwrap(),
        0
    );
    // Anyone else's profile needs the token.
    let hoa = store.add_person("Hoa", 1).unwrap();
    let hp = store
        .put_voice_profile(
            &hoa,
            &consent(),
            None,
            "m1",
            vec![("vi".into(), vec![exemplar(0.2)])],
            approved(),
        )
        .unwrap();
    assert!(store.drop_voice_exemplars_from(&hp, &m, None).is_err());
}

#[test]
fn raw_counts_and_debug_hide_nothing_but_vectors() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let me = store.me_person().unwrap();
    store
        .put_voice_profile(
            &me,
            &consent(),
            None,
            "m1",
            vec![("any".into(), vec![exemplar(0.25)])],
            None,
        )
        .unwrap();
    let hoa = store.add_person("Hoa", 1).unwrap();
    store
        .put_voice_profile(
            &hoa,
            &consent(),
            None,
            "m1",
            vec![("vi".into(), vec![exemplar(0.5)])],
            approved(),
        )
        .unwrap();
    let c = store.raw_voice_counts().unwrap();
    assert_eq!((c.profiles, c.other_profiles), (2, 1));
    assert_eq!(
        (c.embedding_rows, c.other_embedding_rows, c.speaker_voices),
        (2, 1, 0)
    );
    let p = store.voice_profile(&me).unwrap().unwrap();
    let shown = format!("{p:?} {:?}", p.sets[0].exemplars[0]);
    assert!(
        !shown.contains("0.25") && !shown.contains("0.75"),
        "{shown}"
    );
}
