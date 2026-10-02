// SPDX-License-Identifier: Apache-2.0
//! Voice profiles (phase 14c, RT-13): sealed under a per-profile key, wrapped
//! by the key ring, crypto-shredded on delete.

mod common;

use ghi_store::keys::KeyRing;
use ghi_store::store::{NewSpeaker, Store};
use ghi_store::voice::{
    ExemplarSource, MAX_EXEMPLARS, ThirdPartyApproved, VoiceConsent, VoiceExemplar,
};
use ghi_store::{StoreError, db};

const MODEL: &str = "campplus";

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

fn ex(x: f32) -> VoiceExemplar {
    VoiceExemplar {
        vec: vec![x, 0.5, 0.25, -x],
        source: None,
    }
}

fn person(store: &Store, meeting: &str, idx: i64, name: &str) -> String {
    let s = store
        .add_speaker(
            meeting,
            NewSpeaker {
                label_idx: idx,
                ..Default::default()
            },
        )
        .unwrap();
    store.rename_speaker(&s, Some(name)).unwrap();
    store
        .speakers(meeting)
        .unwrap()
        .into_iter()
        .find(|x| x.gid == s)
        .unwrap()
        .person_gid
        .unwrap()
}

fn put(store: &Store, person: &str, lang: &str, xs: &[f32]) -> String {
    store
        .put_voice_profile(
            person,
            &consent(),
            None,
            MODEL,
            vec![(lang.into(), xs.iter().map(|x| ex(*x)).collect())],
            approved(),
        )
        .unwrap()
}

fn raw(dir: &std::path::Path, keys: &common::Keys) -> rusqlite::Connection {
    db::open(&dir.join("ghira.db"), &common::db_key(keys)).unwrap()
}

#[test]
fn a_profile_round_trips_sealed_with_consent_and_clip() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let me = store.me_person().unwrap();
    let src = ExemplarSource {
        meeting_gid: "m".into(),
        t0_ms: 1000,
        t1_ms: 5000,
    };
    let consent = VoiceConsent {
        method: "verbal_clip".into(),
        at_ms: 42,
        text_key: "consent.clip".into(),
        clip: Some(src.clone()),
    };
    let marker = 0.123_456_79_f32;
    let clip = b"RIFF-consent-clip-audio".to_vec();
    let gid = store
        .put_voice_profile(
            &me,
            &consent,
            Some(&clip),
            MODEL,
            vec![
                (
                    "vi".into(),
                    vec![
                        VoiceExemplar {
                            vec: vec![marker, 0.5, 0.0, 0.0],
                            source: Some(src.clone()),
                        },
                        ex(0.3),
                    ],
                ),
                ("any".into(), vec![ex(0.9)]),
            ],
            approved(),
        )
        .unwrap();

    let p = store.voice_profile(&me).unwrap().unwrap();
    assert_eq!(
        (p.gid.as_str(), p.person_gid.as_str(), p.is_me),
        (gid.as_str(), me.as_str(), true)
    );
    assert_eq!(p.consent, consent);
    let vi = p.sets.iter().find(|s| s.lang == "vi").unwrap();
    assert_eq!(vi.exemplars[0].vec[0], marker);
    assert_eq!(vi.exemplars[0].source.as_ref(), Some(&src));
    assert_eq!(vi.exemplars[1].source, None);
    let norm: f32 = vi.centroid.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5, "the centroid is normalised");
    assert_eq!(p.sets.len(), 2);
    assert_eq!(
        store.voice_consent_clip(&gid).unwrap().as_deref(),
        Some(clip.as_slice())
    );
    assert_eq!(
        store.me_voice_profile(MODEL).unwrap().unwrap().sets.len(),
        2
    );
    assert!(
        store
            .me_voice_profile("other-model")
            .unwrap()
            .unwrap()
            .sets
            .is_empty()
    );

    // Survives a reopen (the key is unwrapped from the ring again).
    drop(store);
    let store = common::reopen(tmp.path(), &keys);
    assert_eq!(store.voice_profile(&me).unwrap().unwrap().sets.len(), 2);
    drop(store);

    // On disk: no plaintext floats, no plaintext clip, a wrapped key.
    let conn = raw(tmp.path(), &keys);
    let blobs: Vec<Vec<u8>> = conn
        .prepare("SELECT vec_ct FROM voice_embeddings")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(blobs.len(), 2);
    let needle = marker.to_le_bytes();
    assert!(blobs.iter().all(|b| !b.windows(4).any(|w| w == needle)));
    let (clip_ct, wrapped): (Vec<u8>, Vec<u8>) = conn
        .query_row(
            "SELECT consent_clip_ct, key_wrapped FROM voice_profiles",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(!clip_ct.windows(clip.len()).any(|w| w == clip.as_slice()));
    // The wrapped key opens with the ring and AAD `voice:{gid}`, nothing else.
    let ring = common::ring(&keys);
    assert!(ring.unwrap_voice_key(&wrapped, &gid).is_ok());
    assert!(ring.unwrap_voice_key(&wrapped, "other").is_err());
    assert!(ring.unwrap_dek(&wrapped, &gid).is_err());
}

#[test]
fn exemplars_are_capped_newest_kept_and_the_centroid_follows() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let me = store.me_person().unwrap();
    let gid = put(&store, &me, "vi", &[0.0]);
    let batch: Vec<_> = (1..=25).map(|i| ex(i as f32)).collect();
    store
        .add_voice_exemplars(&gid, MODEL, "vi", batch, approved())
        .unwrap();
    let set = store.voice_profile(&me).unwrap().unwrap().sets.remove(0);
    assert_eq!(set.exemplars.len(), MAX_EXEMPLARS);
    let firsts: Vec<f32> = set.exemplars.iter().map(|e| e.vec[0]).collect();
    assert_eq!(firsts[0], 6.0, "the oldest went");
    assert_eq!(*firsts.last().unwrap(), 25.0);
    // A new language adds a set; empty adds are no-ops; mixed sizes refused.
    store
        .add_voice_exemplars(&gid, MODEL, "en", vec![ex(0.1)], approved())
        .unwrap();
    store
        .add_voice_exemplars(&gid, MODEL, "en", vec![], approved())
        .unwrap();
    assert_eq!(store.voice_profile(&me).unwrap().unwrap().sets.len(), 2);
    let bad = VoiceExemplar {
        vec: vec![1.0],
        source: None,
    };
    assert!(matches!(
        store.add_voice_exemplars(&gid, MODEL, "en", vec![bad], approved()),
        Err(StoreError::Invalid(_))
    ));
    assert!(
        store
            .add_voice_exemplars("nope", MODEL, "en", vec![ex(1.0)], approved())
            .is_err()
    );
}

#[test]
fn third_party_profiles_need_the_flag_checked_token() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let an = person(&store, &m, 0, "An");
    put(&store, &store.me_person().unwrap(), "vi", &[0.1]);
    put(&store, &an, "vi", &[0.2]);
    let me = store.me_voice_profile(MODEL).unwrap().unwrap();
    assert!(me.is_me);
    let others = store
        .third_party_voice_profiles(MODEL, ThirdPartyApproved::assert_flag_checked())
        .unwrap();
    assert_eq!(others.len(), 1);
    assert!(!others[0].is_me);
    assert_eq!(others[0].person_gid, an);
    // Overview carries the consent summary.
    let ov = store.person(&an).unwrap();
    let v = ov.voice.unwrap();
    assert_eq!(
        (v.method.as_str(), v.consent_at_ms),
        ("self_checkbox", 1_700_000_000_000)
    );
}

#[test]
fn replacing_a_profile_shreds_the_old_one() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let me = store.me_person().unwrap();
    let old = put(&store, &me, "vi", &[0.1]);
    let before = common::ring(&keys);
    let old_wrapped: Vec<u8> = raw(tmp.path(), &keys)
        .query_row("SELECT key_wrapped FROM voice_profiles", [], |r| r.get(0))
        .unwrap();
    let new = put(&store, &me, "vi", &[0.7]);
    assert_ne!(old, new);
    assert!(store.is_tombstoned(&old).unwrap());
    let p = store.voice_profile(&me).unwrap().unwrap();
    assert_eq!(p.gid, new);
    assert_eq!(p.sets[0].exemplars[0].vec[0], 0.7);
    let n: i64 = raw(tmp.path(), &keys)
        .query_row("SELECT count(*) FROM voice_profiles", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    // The old key can no longer be unwrapped with the current ring.
    assert!(
        before.unwrap_voice_key(&old_wrapped, &old).is_ok(),
        "control"
    );
    assert!(
        common::ring(&keys)
            .unwrap_voice_key(&old_wrapped, &old)
            .is_err()
    );
}

#[test]
fn delete_shreds_the_key_keeps_names_and_other_profiles() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let an = person(&store, &m, 0, "An");
    let bo = person(&store, &m, 1, "Bo");
    let g_an = put(&store, &an, "vi", &[0.1, 0.2]);
    let g_bo = put(&store, &bo, "vi", &[0.3]);

    // The attacker's copy of the data directory, before the delete, and the
    // ring as it was.
    let copy = tmp.path().join("copy-before");
    let ring_before: KeyRing = common::ring(&keys);
    store.checkpoint().unwrap();
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(tmp.path().join("ghira.db"), copy.join("ghira.db")).unwrap();

    store.delete_voice_profile(&g_an).unwrap();

    // The person and the name in the meeting stay; the profile is gone.
    assert!(store.voice_profile(&an).unwrap().is_none());
    let s = store.speakers(&m).unwrap();
    assert!(
        s.iter().any(|s| s.display_name.as_deref() == Some("An")
            && s.person_gid.as_deref() == Some(an.as_str()))
    );
    assert!(store.is_tombstoned(&g_an).unwrap());
    // The other profile survives the rotation, also after a reopen.
    assert_eq!(store.voice_profile(&bo).unwrap().unwrap().gid, g_bo);
    drop(store);
    let store = common::reopen(tmp.path(), &keys);
    assert_eq!(store.voice_profile(&bo).unwrap().unwrap().gid, g_bo);
    drop(store);

    // Rows are gone from the live database ...
    let conn = raw(tmp.path(), &keys);
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM voice_profiles WHERE gid = ?1",
            [&g_an],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
    let n: i64 = conn
        .query_row("SELECT count(*) FROM voice_embeddings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1, "only Bo's set");
    // ... and the copy taken before holds a wrapped key the current ring
    // can't open, while the ring from before could (control).
    let old = db::open(&copy.join("ghira.db"), &common::db_key(&keys)).unwrap();
    let wrapped: Vec<u8> = old
        .query_row(
            "SELECT key_wrapped FROM voice_profiles WHERE gid = ?1",
            [&g_an],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        ring_before.unwrap_voice_key(&wrapped, &g_an).is_ok(),
        "control"
    );
    assert!(
        common::ring(&keys)
            .unwrap_voice_key(&wrapped, &g_an)
            .is_err()
    );
    assert!(!common::ring(&keys).is_rotating());
}

#[test]
fn a_crash_after_the_key_is_zeroed_is_finished_on_open() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let an = person(&store, &m, 0, "An");
    let g = put(&store, &an, "vi", &[0.1]);

    // Steps 1-3 only: tombstone written, key zeroed (rows still there).
    store.shred_voice_key(&g).unwrap();
    let conn = raw(tmp.path(), &keys);
    let (zeroed, rows): (bool, i64) = conn
        .query_row(
            "SELECT key_wrapped = zeroblob(length(key_wrapped)),
                    (SELECT count(*) FROM voice_embeddings) FROM voice_profiles",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(zeroed && rows == 1, "the key goes first, the rows later");
    drop(conn);
    // Nothing opens the profile any more.
    assert!(matches!(store.voice_profile(&an), Err(StoreError::Decrypt)));
    assert!(store.me_voice_profile(MODEL).unwrap().is_none());
    drop(store);

    let store = common::reopen(tmp.path(), &keys);
    assert!(store.voice_profile(&an).unwrap().is_none());
    let n: i64 = raw(tmp.path(), &keys)
        .query_row("SELECT count(*) FROM voice_embeddings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(
        store.speakers(&m).unwrap()[0].display_name.as_deref(),
        Some("An")
    );
}

#[test]
fn a_crash_before_the_rotation_is_finished_on_open() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let me = store.me_person().unwrap();
    let g = put(&store, &me, "vi", &[0.1]);
    let wrapped: Vec<u8> = raw(tmp.path(), &keys)
        .query_row("SELECT key_wrapped FROM voice_profiles", [], |r| r.get(0))
        .unwrap();
    store.delete_voice_profile_before_rotation(&g).unwrap();
    assert!(common::ring(&keys).is_rotating());
    drop(store);
    let _store = common::reopen(tmp.path(), &keys);
    assert!(!common::ring(&keys).is_rotating());
    assert!(common::ring(&keys).unwrap_voice_key(&wrapped, &g).is_err());
}

#[test]
fn deleting_a_meeting_rotation_re_wraps_voice_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let doomed = common::meeting(&store, "Doomed");
    let an = person(&store, &m, 0, "An");
    let g = put(&store, &an, "vi", &[0.1]);
    let before = common::ring(&keys);
    let wrapped_before: Vec<u8> = raw(tmp.path(), &keys)
        .query_row("SELECT key_wrapped FROM voice_profiles", [], |r| r.get(0))
        .unwrap();
    store.delete_meeting(&doomed).unwrap();
    drop(store);
    let wrapped_after: Vec<u8> = raw(tmp.path(), &keys)
        .query_row("SELECT key_wrapped FROM voice_profiles", [], |r| r.get(0))
        .unwrap();
    assert_ne!(wrapped_before, wrapped_after);
    assert!(before.unwrap_voice_key(&wrapped_before, &g).is_ok());
    assert!(
        common::ring(&keys)
            .unwrap_voice_key(&wrapped_before, &g)
            .is_err()
    );
    let store = common::reopen(tmp.path(), &keys);
    assert_eq!(store.voice_profile(&an).unwrap().unwrap().gid, g);
}

#[test]
fn sealed_vectors_are_bound_to_their_profile_model_and_language() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let an = person(&store, &m, 0, "An");
    let bo = person(&store, &m, 1, "Bo");
    let g_an = put(&store, &an, "vi", &[0.1]);
    put(&store, &bo, "vi", &[0.9]);
    store
        .add_voice_exemplars(&g_an, MODEL, "en", vec![ex(0.4)], approved())
        .unwrap();
    let conn = raw(tmp.path(), &keys);
    let take = |profile: &str, lang: &str| -> Vec<u8> {
        conn.query_row(
            "SELECT e.vec_ct FROM voice_embeddings e JOIN voice_profiles v ON v.id = e.profile_id
             JOIN persons p ON p.id = v.person_id WHERE p.name = ?1 AND e.lang = ?2",
            [profile, lang],
            |r| r.get(0),
        )
        .unwrap()
    };

    // Bo's blob moved into An's row: Bo's key, An's profile: rejected.
    let (an_vi, bo_vi, an_en) = (take("An", "vi"), take("Bo", "vi"), take("An", "en"));
    let set = |profile: &str, lang: &str, ct: &[u8]| {
        conn.execute(
            "UPDATE voice_embeddings SET vec_ct = ?3
             WHERE lang = ?2 AND profile_id =
                (SELECT v.id FROM voice_profiles v JOIN persons p ON p.id = v.person_id WHERE p.name = ?1)",
            rusqlite::params![profile, lang, ct],
        )
        .unwrap();
    };
    drop(store);
    set("An", "vi", &bo_vi);
    let store = common::reopen(tmp.path(), &keys);
    assert!(matches!(store.voice_profile(&an), Err(StoreError::Decrypt)));
    drop(store);
    // Same profile, other language: same key, but the AAD binds the language.
    set("An", "vi", &an_en);
    let store = common::reopen(tmp.path(), &keys);
    assert!(matches!(store.voice_profile(&an), Err(StoreError::Decrypt)));
    drop(store);
    // The original blob works again.
    set("An", "vi", &an_vi);
    let store = common::reopen(tmp.path(), &keys);
    assert!(store.voice_profile(&an).is_ok());
    // A damaged blob is rejected too.
    let mut bad = an_vi.clone();
    *bad.last_mut().unwrap() ^= 1;
    drop(store);
    set("An", "vi", &bad);
    let store = common::reopen(tmp.path(), &keys);
    assert!(matches!(store.voice_profile(&an), Err(StoreError::Decrypt)));
}

#[test]
fn speaker_voices_are_sealed_under_the_meeting_and_go_with_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let s1 = store
        .add_speaker(
            &m,
            NewSpeaker {
                label_idx: 0,
                ..Default::default()
            },
        )
        .unwrap();
    let s2 = store
        .add_speaker(
            &m,
            NewSpeaker {
                label_idx: 1,
                ..Default::default()
            },
        )
        .unwrap();
    let marker = 0.314_159_27_f32;
    store
        .put_speaker_voice(
            &s1,
            MODEL,
            "vi",
            &[marker, 0.1, 0.2],
            ThirdPartyApproved::assert_flag_checked(),
        )
        .unwrap();
    store
        .put_speaker_voice(
            &s2,
            MODEL,
            "en",
            &[0.5, 0.5],
            ThirdPartyApproved::assert_flag_checked(),
        )
        .unwrap();
    store
        .put_speaker_voice(
            &s1,
            MODEL,
            "vi",
            &[marker, 0.1, 0.3],
            ThirdPartyApproved::assert_flag_checked(),
        )
        .unwrap();
    let v = store.speaker_voice(&s1).unwrap().unwrap();
    assert_eq!(
        (v.model.as_str(), v.lang.as_str(), v.vec.clone()),
        (MODEL, "vi", vec![marker, 0.1, 0.3])
    );
    let all = store.speaker_voices(&m).unwrap();
    assert_eq!(
        all.iter().map(|(g, _)| g.as_str()).collect::<Vec<_>>(),
        [s1.as_str(), s2.as_str()]
    );
    assert!(
        store
            .put_speaker_voice(
                &s1,
                MODEL,
                "vi",
                &[],
                ThirdPartyApproved::assert_flag_checked()
            )
            .is_err()
    );
    store.clear_speaker_voice(&s2).unwrap();
    assert!(store.speaker_voice(&s2).unwrap().is_none());

    store.checkpoint().unwrap();
    let blob: Vec<u8> = raw(tmp.path(), &keys)
        .query_row("SELECT vec_ct FROM speaker_voices", [], |r| r.get(0))
        .unwrap();
    assert!(!blob.windows(4).any(|w| w == marker.to_le_bytes()));

    store.delete_meeting(&m).unwrap();
    let n: i64 = raw(tmp.path(), &keys)
        .query_row("SELECT count(*) FROM speaker_voices", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

// ------------------------------------------------------- review follow-ups

#[test]
fn only_me_needs_no_token_and_consent_is_validated() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let an = person(&store, &m, 0, "An");
    let me = store.me_person().unwrap();
    let sets = || vec![("vi".to_string(), vec![ex(0.1)])];
    // Me: no token. Anyone else: refused without one, and nothing is written.
    let g = store
        .put_voice_profile(&me, &consent(), None, MODEL, sets(), None)
        .unwrap();
    assert!(matches!(
        store.put_voice_profile(&an, &consent(), None, MODEL, sets(), None),
        Err(StoreError::Invalid(_))
    ));
    assert!(store.voice_profile(&an).unwrap().is_none());
    let ga = put(&store, &an, "vi", &[0.2]);
    assert!(matches!(
        store.add_voice_exemplars(&ga, MODEL, "vi", vec![ex(0.3)], None),
        Err(StoreError::Invalid(_))
    ));
    store
        .add_voice_exemplars(&g, MODEL, "vi", vec![ex(0.3)], None)
        .unwrap();

    // D10: known method; a verbal clip needs its audio.
    let bad = VoiceConsent {
        method: "oral".into(),
        ..consent()
    };
    let verbal = VoiceConsent {
        method: "verbal_clip".into(),
        ..consent()
    };
    for (c, clip) in [(&bad, None), (&verbal, None), (&verbal, Some(&b""[..]))] {
        assert!(matches!(
            store.put_voice_profile(&me, c, clip, MODEL, sets(), None),
            Err(StoreError::Invalid(_))
        ));
    }
    assert_eq!(
        store.voice_profile(&me).unwrap().unwrap().gid,
        g,
        "untouched"
    );
    assert!(
        store
            .put_voice_profile(&me, &verbal, Some(b"clip"), MODEL, sets(), None)
            .is_ok()
    );
}

#[test]
fn a_refused_replace_leaves_the_old_profile_and_the_ring_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let me = store.me_person().unwrap();
    let old = put(&store, &me, "vi", &[0.1]);
    let ragged = vec![
        ex(0.1),
        VoiceExemplar {
            vec: vec![1.0],
            source: None,
        },
    ];
    assert!(
        store
            .put_voice_profile(
                &me,
                &consent(),
                None,
                MODEL,
                vec![("vi".into(), ragged)],
                None
            )
            .is_err()
    );
    assert_eq!(store.voice_profile(&me).unwrap().unwrap().gid, old);
    assert!(!store.is_tombstoned(&old).unwrap());
    assert!(
        !common::ring(&keys).is_rotating(),
        "no rotation was started"
    );
}

#[test]
fn a_replace_is_one_step_and_leaves_one_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let me = store.me_person().unwrap();
    let old = put(&store, &me, "vi", &[0.1]);
    let new = put(&store, &me, "vi", &[0.6]);
    drop(store);
    let conn = raw(tmp.path(), &keys);
    let rows: Vec<(String, bool)> = conn
        .prepare("SELECT gid, key_wrapped = zeroblob(length(key_wrapped)) FROM voice_profiles")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows, vec![(new.clone(), false)], "no zeroed leftover");
    let n: i64 = conn
        .query_row("SELECT count(*) FROM voice_embeddings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    drop(conn);
    let store = common::reopen(tmp.path(), &keys);
    assert!(store.is_tombstoned(&old).unwrap());
    assert!(!common::ring(&keys).is_rotating());
}

#[test]
fn a_profile_written_while_a_rotation_is_open_survives_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let me = store.me_person().unwrap();
    // A rotation that crashed after the new secret was saved: the new profile
    // must be wrapped by the current secret, so finishing it keeps it.
    store.begin_rotation_only().unwrap();
    let g = put(&store, &me, "vi", &[0.4]);
    drop(store);
    let store = common::reopen(tmp.path(), &keys);
    assert!(!common::ring(&keys).is_rotating());
    assert_eq!(store.voice_profile(&me).unwrap().unwrap().gid, g);
}

#[test]
fn profiles_written_during_meeting_deletes_stay_readable() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let store = std::sync::Arc::new(store);
    let me = store.me_person().unwrap();
    let doomed: Vec<String> = (0..6)
        .map(|i| common::meeting(&store, &format!("M{i}")))
        .collect();
    let deleter = {
        let store = store.clone();
        std::thread::spawn(move || {
            for g in doomed {
                store.delete_meeting(&g).unwrap();
            }
        })
    };
    let mut last = String::new();
    for i in 0..12 {
        last = put(&store, &me, "vi", &[i as f32 / 20.0]);
    }
    deleter.join().unwrap();
    assert_eq!(store.voice_profile(&me).unwrap().unwrap().gid, last);
    drop(store);
    let store = common::reopen(tmp.path(), &keys);
    assert_eq!(store.voice_profile(&me).unwrap().unwrap().gid, last);
}
