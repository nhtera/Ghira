// SPDX-License-Identifier: Apache-2.0
//! Every shipped schema version (v1 .. latest) has a small SQLCipher fixture,
//! `tests/fixtures/schema/vN.db`. Each one opens with today's code, upgrades
//! to the latest schema and keeps its data.
//!
//! What is in every fixture (rows as that version could hold them):
//! - meeting `m1` ("Họp kế hoạch quý bốn", ready): three sealed transcript
//!   lines (with word timings and folded FTS rows), three speakers (Me, and
//!   two that name the same person: v1-v5 hold that person twice, which the
//!   v6 step must merge), a sealed note block, an action item, a mark, a job,
//!   a cloud request, a setting, and per version: `source_hash` / a discard
//!   (v3), a waveform (v4), two embeddings (v5), a voice profile (v6), and a
//!   folder, tags, source app, calendar info, track speakers and an overlap
//!   mark (v7), a final-pass checkpoint (v8), and `ord` on the note and the
//!   action item (v9; its sync tables are empty and `sync_log` holds what the
//!   triggers logged);
//! - `m2`: deleted for good (only its tombstone is left);
//! - `m3`: crypto-shredded but not yet removed (a delete a crash interrupted;
//!   its key is zeroed, its rows and FTS tokens are still there).
//!
//! Test key ring only ([`common::fixture_ring`]); no real key or data.
//!
//! Rebuilding: the old APIs are gone, so `regenerate_fixtures` applies
//! `MIGRATIONS[..N]` to an empty database and writes the rows with SQL as
//! schema N has them, sealing text with the real `rowcrypt`. v7's
//! folder/tag/calendar/track-speaker rows go through the public
//! `Store` API (which migrates to the latest schema, so only the newest fixture
//! can be rebuilt that way: v1-v8 are committed as they were). When v10 ships,
//! add a `seed` step for v10, pin the v7 extras to raw SQL if the API moves on,
//! and rebuild just the new one:
//!
//! ```sh
//! GHI_FIXTURE_ONLY=9 cargo test -p ghi-store --test schema_upgrade -- --ignored regenerate_fixtures
//! ```

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use ghi_store::embeddings::EmbeddingChunk;
use ghi_store::keys::{KeyRing, Protection};
use ghi_store::migrate::{self, MIGRATIONS};
use ghi_store::organize::TrackSpeaker;
use ghi_store::rowcrypt::{Dek, ROWS_INFO, row_aad, seal, seal_text};
use ghi_store::search::{HitKind, SearchQuery};
use ghi_store::store::{MarkTag, Provenance, Store};
use ghi_store::{StoreError, db, fold};
use rusqlite::{Connection, ToSql, params};

use common::{FIXTURE_VERSIONS, gid, m1, m2, m3};

const MODEL: &str = "test-embed";
/// v8's final-pass checkpoint of m1 (what a pass that yielded left).
const CKPT_STAMP: &str = "fixture-stamp";
const CKPT_PART: &str = "asr.0.0";
const CKPT_DATA: &[u8] = r#"[[{"t":"chốt","s":0.0,"e":0.5,"c":0.9,"k":null}]]"#.as_bytes();
/// The fixtures stay small: they are committed binaries (SQLCipher pages are
/// 4 KiB and every table and index takes at least one, so v7 is ~275 KiB and
/// v9, with the sync tables and triggers, ~370 KiB).
const MAX_FIXTURE_BYTES: u64 = 400 * 1024;

fn meeting_dek(n: u8) -> Dek {
    Dek::from_bytes([0x40 + n; 32])
}

fn vec_of(chunk: u32) -> Vec<f32> {
    let c = chunk as f32;
    vec![0.25 + c, 0.5, -0.25 - c, 1.0 / 3.0]
}

// ---------------------------------------------------------------- generator

/// `INSERT INTO table (cols) VALUES (...)`; returns the rowid.
fn insert(conn: &Connection, table: &str, cols: &[(&str, &dyn ToSql)]) -> i64 {
    let names: Vec<&str> = cols.iter().map(|c| c.0).collect();
    let marks: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
    conn.execute(
        &format!(
            "INSERT INTO {table} ({}) VALUES ({})",
            names.join(", "),
            marks.join(", ")
        ),
        rusqlite::params_from_iter(cols.iter().map(|c| c.1)),
    )
    .unwrap_or_else(|e| panic!("insert into {table}: {e}"));
    conn.last_insert_rowid()
}

#[allow(clippy::too_many_arguments)]
fn insert_meeting(
    conn: &Connection,
    v: u32,
    ring: &KeyRing,
    gid: &str,
    dek: &Dek,
    title: &str,
    status: &str,
    started_at: i64,
) -> i64 {
    let title_ct = seal_text(dek, title, &row_aad("meetings", "title_ct", gid));
    let wrapped = ring.wrap_dek(dek, gid);
    let duration = 600_000i64;
    let lamport = 1i64;
    let source_hash = "00".repeat(32);
    let mut cols: Vec<(&str, &dyn ToSql)> = vec![
        ("gid", &gid),
        ("title_ct", &title_ct),
        ("started_at", &started_at),
        ("duration_ms", &duration),
        ("lang", &"vi"),
        ("status", &status),
        ("dek_wrapped", &wrapped),
        ("lamport", &lamport),
    ];
    if v >= 3 {
        cols.push(("source_hash", &source_hash));
    }
    if v >= 4 {
        cols.push(("created_at", &started_at));
    }
    insert(conn, "meetings", &cols)
}

fn insert_segment(
    conn: &Connection,
    dek: &Dek,
    meeting_id: i64,
    speaker_id: Option<i64>,
    idx: i64,
    gid_no: u16,
    text: &str,
) -> (String, i64) {
    let gid = gid(3, gid_no);
    let text_ct = seal_text(dek, text, &row_aad("segments", "text_ct", &gid));
    let (t0, t1) = (idx * 5000, idx * 5000 + 4000);
    let (version, lamport, confidence) = (1i64, 2 + idx, 0.9f64);
    let id = insert(
        conn,
        "segments",
        &[
            ("gid", &gid),
            ("meeting_id", &meeting_id),
            ("version", &version),
            ("speaker_id", &speaker_id),
            ("t0_ms", &t0),
            ("t1_ms", &t1),
            ("text_ct", &text_ct),
            ("lang", &"vi"),
            ("confidence", &confidence),
            ("lamport", &lamport),
        ],
    );
    conn.execute(
        "INSERT INTO segments_fts (rowid, text_norm) VALUES (?1, ?2)",
        params![id, fold::fold(&fold::nfc(text))],
    )
    .unwrap();
    (gid, id)
}

/// Writes the rows of schema version `v` (see the module docs). Does not shred
/// `m3`: [`finish_fixture`] does, after v7's `Store` API calls.
fn seed(conn: &Connection, v: u32, ring: &KeyRing) {
    let dek1 = meeting_dek(1);
    let dek3 = meeting_dek(3);
    let started = 1_700_000_000_000i64;
    let m1_id = insert_meeting(
        conn,
        v,
        ring,
        &m1(),
        &dek1,
        "Họp kế hoạch quý bốn",
        "ready",
        started,
    );
    let m3_id = insert_meeting(
        conn,
        v,
        ring,
        &m3(),
        &dek3,
        "Cuộc họp bí mật",
        "ready",
        started + 1,
    );

    // People. v1-v5 hold "Bình" twice (the v6 step merges them). v6+ has Me.
    let color = 1i64;
    let person = |g: String, name: &str| -> i64 {
        let key = ghi_store::people::name_key(name);
        let created = started;
        let mut cols: Vec<(&str, &dyn ToSql)> = vec![
            ("gid", &g),
            ("name", &name),
            ("color_slot", &color),
            ("lamport", &1i64),
        ];
        if v >= 6 {
            cols.push(("name_key", &key));
            cols.push(("created_at", &created));
        }
        insert(conn, "persons", &cols)
    };
    let binh = person(gid(7, 1), "Trần Thị Bình");
    let _cuong = person(gid(7, 3), "Lê Văn Cường");
    let binh_again = (v < 6).then(|| person(gid(7, 2), "  trần thị bình "));
    let me_person: Option<i64> = (v >= 6).then(|| {
        conn.query_row("SELECT id FROM persons WHERE is_me = 1", [], |r| r.get(0))
            .unwrap()
    });

    let speaker = |g: String, idx: i64, name: Option<&str>, person: Option<i64>, me: bool| -> i64 {
        let name_ct =
            name.map(|n| seal_text(&dek1, n, &row_aad("speakers", "display_name_ct", &g)));
        insert(
            conn,
            "speakers",
            &[
                ("gid", &g),
                ("meeting_id", &m1_id),
                ("label_idx", &idx),
                ("display_name_ct", &name_ct),
                ("person_id", &person),
                ("color_slot", &idx),
                ("is_me", &me),
                ("lamport", &1i64),
            ],
        )
    };
    let s_me = speaker(gid(2, 1), 0, None, me_person, true);
    let s_binh = speaker(gid(2, 2), 1, Some("Bình"), Some(binh), false);
    let s_binh2 = speaker(
        gid(2, 3),
        2,
        Some("Bình"),
        Some(binh_again.unwrap_or(binh)),
        false,
    );

    // Transcript + word timings.
    let speakers = [s_me, s_binh, s_binh2];
    let mut seg_ids = Vec::new();
    for (i, text) in common::M1_LINES.iter().enumerate() {
        seg_ids.push(insert_segment(
            conn,
            &dek1,
            m1_id,
            Some(speakers[i]),
            i as i64,
            i as u16 + 1,
            text,
        ));
    }
    for w in 0..3i64 {
        insert(
            conn,
            "words",
            &[
                ("segment_id", &seg_ids[0].1),
                ("idx", &w),
                ("t0_ms", &(w * 1000)),
                ("t1_ms", &(w * 1000 + 900)),
                ("conf", &0.8f64),
            ],
        );
    }
    insert_segment(conn, &dek3, m3_id, None, 0, 9, "Nội dung bí mật Zebra");

    // Notes (+ FTS), action item, mark.
    let note_gid = gid(4, 1);
    let note = "Quyết định: chốt ngân sách quý bốn";
    let note_ct = seal_text(&dek1, note, &row_aad("notes_blocks", "body_ct", &note_gid));
    let mut note_cols: Vec<(&str, &dyn ToSql)> = vec![
        ("gid", &note_gid),
        ("meeting_id", &m1_id),
        ("kind", &"summary"),
        ("provenance", &"ai"),
        ("body_ct", &note_ct),
        ("anchors_json", &"[]"),
        ("lamport", &3i64),
    ];
    if v >= 9 {
        note_cols.push(("ord", &"0001"));
    }
    let note_id = insert(conn, "notes_blocks", &note_cols);
    conn.execute(
        "INSERT INTO notes_fts (rowid, body_norm) VALUES (?1, ?2)",
        params![note_id, fold::fold(note)],
    )
    .unwrap();

    let action_gid = gid(5, 1);
    let anchor = format!(
        r#"{{"meeting_gid":"{}","t0_ms":5000,"t1_ms":9000,"transcript_version":1}}"#,
        m1()
    );
    let anchors = format!("[{anchor}]");
    let text_ct = seal_text(
        &dek1,
        "Gửi báo cáo ngân sách",
        &row_aad("action_items", "text_ct", &action_gid),
    );
    let due_ct = seal_text(
        &dek1,
        "thứ Sáu",
        &row_aad("action_items", "due_text_ct", &action_gid),
    );
    let mut cols: Vec<(&str, &dyn ToSql)> = vec![
        ("gid", &action_gid),
        ("meeting_id", &m1_id),
        ("text_ct", &text_ct),
        ("owner_speaker_id", &s_binh),
        ("done", &false),
        ("anchor_json", &anchor),
        ("lamport", &4i64),
    ];
    if v >= 2 {
        cols.push(("provenance", &"ai"));
        cols.push(("due_text_ct", &due_ct));
        cols.push(("anchors_json", &anchors));
    }
    if v >= 9 {
        cols.push(("ord", &"0001"));
    }
    insert(conn, "action_items", &cols);
    insert(
        conn,
        "marks",
        &[
            ("gid", &gid(6, 1)),
            ("meeting_id", &m1_id),
            ("t_ms", &4200i64),
            ("tag", &"decision"),
            ("lamport", &5i64),
        ],
    );

    // Version-specific tables.
    if v >= 3 {
        insert(
            conn,
            "discards",
            &[
                ("meeting_id", &m1_id),
                ("t0_ms", &20_000i64),
                ("t1_ms", &30_000i64),
                ("keep_pages_json", &r#"{"mic":3}"#),
                ("audio_state", &"done"),
                ("created_at", &started),
            ],
        );
    }
    if v >= 4 {
        let ct = seal(
            &dek1,
            &[1, 2, 3, 4],
            &row_aad("waveforms", "data_ct", &m1()),
        );
        insert(
            conn,
            "waveforms",
            &[("meeting_id", &m1_id), ("data_ct", &ct)],
        );
    }
    if v >= 8 {
        let ct = seal(
            &dek1,
            CKPT_DATA,
            &row_aad(
                "final_pass_ckpt",
                "data_ct",
                &format!("{}:{CKPT_STAMP}:{CKPT_PART}", m1()),
            ),
        );
        insert(
            conn,
            "final_pass_ckpt",
            &[
                ("meeting_id", &m1_id),
                ("part", &CKPT_PART),
                ("stamp", &CKPT_STAMP),
                ("data_ct", &ct),
            ],
        );
    }
    if v >= 5 {
        let key = dek1.subkey(ROWS_INFO);
        for chunk in 0..2u32 {
            let plain: Vec<u8> = vec_of(chunk).iter().flat_map(|x| x.to_le_bytes()).collect();
            let aad = format!("embeddings.vec_ct:{}:{chunk}:{MODEL}", m1());
            let ct = seal(&key, &plain, aad.as_bytes());
            let (t0, t1) = (i64::from(chunk) * 60_000, i64::from(chunk + 1) * 60_000);
            let gen0 = 0i64;
            let mut cols: Vec<(&str, &dyn ToSql)> = vec![
                ("meeting_id", &m1_id),
                ("chunk", &chunk),
                ("t0_ms", &t0),
                ("t1_ms", &t1),
                ("transcript_version", &1i64),
                ("model", &MODEL),
                ("dim", &4i64),
                ("vec_ct", &ct),
            ];
            if v >= 6 {
                cols.push(("index_gen", &gen0));
            }
            insert(conn, "embeddings", &cols);
        }
    }
    if v >= 6 {
        let profile_gid = gid(8, 1);
        let vkey = Dek::from_bytes([0x58; 32]);
        let clip = seal(
            &vkey.subkey(b"ghira/voice/v1"),
            b"clip",
            format!("voice_profiles.consent_clip_ct:{profile_gid}").as_bytes(),
        );
        let profile_id = insert(
            conn,
            "voice_profiles",
            &[
                ("gid", &profile_gid),
                ("person_id", &binh),
                ("is_me", &false),
                (
                    "consent_json",
                    &r#"{"method":"self_checkbox","consent_at_ms":1700000000000}"#,
                ),
                ("key_wrapped", &ring.wrap_voice_key(&vkey, &profile_gid)),
                ("consent_clip_ct", &clip),
                ("created_at", &started),
                ("updated_at", &started),
                ("lamport", &1i64),
            ],
        );
        insert(
            conn,
            "voice_embeddings",
            &[
                ("profile_id", &profile_id),
                ("model", &"test-voice"),
                ("lang", &"vi"),
                ("dim", &4i64),
                ("n", &1i64),
                (
                    "vec_ct",
                    &seal(&vkey.subkey(b"ghira/voice/v1"), &[0u8; 16], b"test"),
                ),
            ],
        );
    }

    // Meeting-less / shared tables, and the deletes.
    insert(
        conn,
        "jobs",
        &[
            ("meeting_id", &m1_id),
            ("kind", &"final_pass"),
            ("state", &"done"),
            ("progress", &1.0f64),
        ],
    );
    insert(
        conn,
        "cloud_requests",
        &[
            ("meeting_id", &m1_id),
            ("provider", &"test"),
            ("model", &"test-model"),
            ("tokens_in", &100i64),
            ("tokens_out", &20i64),
            ("at", &started),
        ],
    );
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value_json) VALUES ('theme', '\"dark\"')",
        [],
    )
    .unwrap();
    for (g, lamport) in [(m2(), 7i64), (m3(), 8)] {
        insert(
            conn,
            "tombstones",
            &[
                ("gid", &g),
                ("kind", &"meeting"),
                ("lamport", &lamport),
                ("deleted_at", &started),
            ],
        );
    }
    conn.execute(
        "UPDATE settings SET value_json = '40' WHERE key = 'lamport'",
        [],
    )
    .unwrap();
}

/// v7's rows are the current schema's: written with the public API.
fn seed_v7_extras(dir: &Path) {
    let store = Store::open(
        dir,
        common::keys_with(common::fixture_ring()),
        Protection::default(),
    )
    .unwrap();
    let m = [m1()];
    let folder = store.create_folder("Họp nhóm").unwrap();
    store.set_meeting_folder(&m, Some(&folder.gid)).unwrap();
    for name in ["Quan trọng", "Q4"] {
        let tag = store.create_tag(name).unwrap();
        store.tag_meetings(&m, &tag.gid).unwrap();
    }
    store.set_source_app(&m1(), Some("zoom")).unwrap();
    store
        .set_calendar_info(
            &m1(),
            Some(&serde_json::json!({"title": "Kế hoạch Q4", "attendees": ["Bình"]})),
        )
        .unwrap();
    store
        .set_track_speakers(
            &m1(),
            &[TrackSpeaker {
                label: "Bình".into(),
                speaker_gid: gid(2, 2),
                spans: vec![[5000, 9000]],
            }],
        )
        .unwrap();
    store.mark_overlaps(&m1(), &[gid(3, 2)]).unwrap();
}

fn build_fixture(v: u32) -> std::path::PathBuf {
    let tmp = tempfile::tempdir().unwrap();
    let ring = common::fixture_ring();
    let path = tmp.path().join("ghira.db");
    let conn = db::open(&path, &ring.db_key()).unwrap();
    let conn = migrate::run(
        conn,
        &path,
        &MIGRATIONS[..v as usize],
        &tmp.path().join("snapshots"),
        migrate::KEEP_SNAPSHOTS,
    )
    .unwrap();
    seed(&conn, v, &ring);
    drop(conn);
    if v >= 7 {
        seed_v7_extras(tmp.path());
    }
    // The interrupted delete: shred m3's key, leave its rows.
    let conn = db::open(&path, &ring.db_key()).unwrap();
    conn.execute(
        "UPDATE meetings SET dek_wrapped = zeroblob(length(dek_wrapped)) WHERE gid = ?1",
        [m3()],
    )
    .unwrap();
    conn.execute_batch("VACUUM;").unwrap();
    db::checkpoint(&conn).unwrap();
    drop(conn);
    let out = common::fixture_file(v);
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    std::fs::copy(&path, &out).unwrap();
    out
}

#[test]
#[ignore = "rewrites tests/fixtures/schema/*.db"]
fn regenerate_fixtures() {
    // GHI_FIXTURE_ONLY=9 rewrites just that one (the others are committed).
    let only: Option<u32> = std::env::var("GHI_FIXTURE_ONLY")
        .ok()
        .and_then(|v| v.parse().ok());
    for v in FIXTURE_VERSIONS.filter(|v| only.is_none_or(|o| o == *v)) {
        let out = build_fixture(v);
        eprintln!("v{v}: {} bytes", std::fs::metadata(out).unwrap().len());
    }
}

// -------------------------------------------------------------------- tests

/// Row count of every table (the FTS ones are checked by searching).
fn table_counts(conn: &Connection) -> BTreeMap<String, i64> {
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    names
        .into_iter()
        .filter(|n| !n.contains("_fts"))
        .map(|n| {
            let c = conn
                .query_row(&format!("SELECT count(*) FROM {n}"), [], |r| r.get(0))
                .unwrap();
            (n, c)
        })
        .collect()
}

fn settings(conn: &Connection) -> Vec<(String, String)> {
    conn.prepare("SELECT key, value_json FROM settings ORDER BY key")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn assert_sound(conn: &Connection) {
    let check: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    let bad = conn
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .count();
    assert_eq!(bad, 0, "foreign key violations");
}

#[test]
fn the_fixtures_are_small_and_at_their_version() {
    let ring = common::fixture_ring();
    for v in FIXTURE_VERSIONS {
        let size = std::fs::metadata(common::fixture_file(v)).unwrap().len();
        assert!(size < MAX_FIXTURE_BYTES, "v{v} fixture is {size} bytes");
        let tmp = tempfile::tempdir().unwrap();
        common::install_fixture(v, tmp.path());
        let conn = db::open(&tmp.path().join("ghira.db"), &ring.db_key()).unwrap();
        assert_eq!(db::user_version(&conn).unwrap(), v);
        assert_sound(&conn);
        // The fixture still holds what it was built with, readable with the
        // test ring alone.
        assert_eq!(common::raw_m1_lines(&conn, &ring), common::M1_LINES);
    }
}

#[test]
fn fixtures_cover_every_shipped_version() {
    assert_eq!(*FIXTURE_VERSIONS.end(), migrate::latest_version());
}

fn upgrade_and_check(v: u32) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let keys = common::install_fixture(v, dir);
    let ring0 = common::ring(&keys);
    let latest = migrate::latest_version();
    let db_path = dir.join("ghira.db");

    // --- first open: upgrades
    let store = common::reopen(dir, &keys);
    assert!(
        migrate::list_snapshots(&dir.join("snapshots"))
            .unwrap()
            .is_empty(),
        "snapshots are purged after a successful upgrade"
    );

    // Meeting, transcript, speakers, notes, actions, marks (sealed rows decrypt).
    let meetings = store.list_meetings(10, 0).unwrap();
    assert_eq!(
        meetings.len(),
        1,
        "only m1 survives: m2 is gone, m3 is finished off"
    );
    let m = &meetings[0];
    assert_eq!(m.gid, m1());
    assert_eq!(m.title, "Họp kế hoạch quý bốn");
    assert_eq!(m.status, "ready");
    assert_eq!(m.lang.as_deref(), Some("vi"));
    assert_eq!(m.transcript_version, 1);
    let segs = store.segments(&m1()).unwrap();
    assert_eq!(
        segs.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
        common::M1_LINES
    );
    assert_eq!(store.segment_words(&segs[0].gid).unwrap().len(), 3);
    let speakers = store.speakers(&m1()).unwrap();
    assert_eq!(speakers.len(), 3);
    assert_eq!(speakers[1].display_name.as_deref(), Some("Bình"));
    assert!(speakers[0].is_me);
    assert_eq!(
        speakers[1].person_gid, speakers[2].person_gid,
        "both Bình speakers are one person (v1-v5: merged by the v6 step)"
    );
    let notes = store.note_blocks(&m1()).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].body, "Quyết định: chốt ngân sách quý bốn");
    let actions = store.action_items(&m1()).unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].text, "Gửi báo cáo ngân sách");
    assert_eq!(
        actions[0].owner_speaker_gid.as_deref(),
        Some(speakers[1].gid.as_str())
    );
    assert_eq!(
        actions[0].anchors.len(),
        1,
        "v1's one citation is in the list"
    );
    if v >= 2 {
        assert_eq!(actions[0].provenance, Provenance::Ai);
        assert_eq!(actions[0].due_text.as_deref(), Some("thứ Sáu"));
    } else {
        assert_eq!(actions[0].provenance, Provenance::User);
    }
    let marks = store.marks(&m1()).unwrap();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].tag, MarkTag::Decision);
    assert_eq!(
        store.waveform(&m1()).unwrap(),
        (v >= 4).then(|| vec![1, 2, 3, 4])
    );

    // Deleted meetings stay gone, including the half-deleted one.
    for g in [m2(), m3()] {
        assert!(matches!(
            store.get_meeting(&g),
            Err(StoreError::NotFound { .. })
        ));
        assert!(store.is_tombstoned(&g).unwrap(), "tombstone kept");
    }

    // VN-folded search: accents optional, typed with or without them.
    for q in ["ke hoach quy bon", "kế hoạch quý bốn", "KE HOACH"] {
        let hits = store.search(&SearchQuery::new(q)).unwrap();
        assert!(
            hits.iter().any(|h| h.kind == HitKind::Segment
                && h.meeting_gid == m1()
                && h.item_gid == segs[0].gid),
            "{q}: {hits:?}"
        );
    }
    let hits = store.search(&SearchQuery::new("ngan sach")).unwrap();
    assert!(
        hits.iter()
            .any(|h| h.kind == HitKind::Segment && h.item_gid == segs[1].gid)
    );
    assert!(
        hits.iter()
            .any(|h| h.kind == HitKind::Note && h.item_gid == notes[0].gid)
    );
    assert!(
        store.search(&SearchQuery::new("zebra")).unwrap().is_empty(),
        "m3's tokens are gone from the index"
    );

    // The delete of m3 rotated the wrap secret; m1 was re-wrapped and still opens.
    let ring1 = common::ring(&keys);
    assert_ne!(ring0, ring1, "finishing m3's delete rotated the ring");
    assert!(!ring1.is_rotating());
    // The Lamport clock follows a remote value, and never goes back.
    store.observe_lamport(10_000).unwrap();
    store.observe_lamport(5).unwrap();
    drop(store);

    // Raw view: version, shape, integrity.
    let conn = db::open(&db_path, &ring1.db_key()).unwrap();
    assert_eq!(db::user_version(&conn).unwrap(), latest);
    assert_sound(&conn);
    assert_eq!(common::raw_m1_lines(&conn, &ring1), common::M1_LINES);
    let counts = table_counts(&conn);
    assert_eq!(counts["meetings"], 1);
    assert_eq!(counts["segments"], 3);
    assert_eq!(counts["words"], 3);
    assert_eq!(counts["speakers"], 3);
    assert_eq!(counts["notes_blocks"], 1);
    assert_eq!(counts["action_items"], 1);
    assert_eq!(counts["marks"], 1);
    assert_eq!(counts["jobs"], 1);
    assert_eq!(counts["cloud_requests"], 1);
    assert_eq!(counts["embeddings"], if v >= 5 { 2 } else { 0 });
    assert_eq!(counts["discards"], i64::from(v >= 3));
    assert_eq!(counts["waveforms"], i64::from(v >= 4));
    assert_eq!(counts["voice_profiles"], i64::from(v >= 6));
    assert_eq!(counts["voice_embeddings"], i64::from(v >= 6));
    assert_eq!(counts["final_pass_ckpt"], i64::from(v >= 8));
    assert_eq!(counts["folders"], i64::from(v >= 7));
    assert_eq!(counts["tags"], if v >= 7 { 2 } else { 0 });
    assert_eq!(counts["meeting_tags"], if v >= 7 { 2 } else { 0 });
    assert_eq!(
        counts["tombstones"],
        2 + i64::from(v < 6),
        "m2, m3 (+ the merged person)"
    );
    // v1-v3 meetings got `created_at = started_at` from the v4 step.
    assert_eq!(
        scalar(&conn, "SELECT created_at = started_at FROM meetings"),
        1
    );
    // People: Bình (once), Cường, and Me; Me's speaker is linked to it.
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM persons WHERE is_me = 0"),
        2
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM persons WHERE is_me = 1"),
        1
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT person_id = (SELECT id FROM persons WHERE is_me = 1)
             FROM speakers WHERE is_me = 1"
        ),
        1
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM persons WHERE name_key IS NULL AND is_me = 0"
        ),
        0
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM meetings WHERE index_gen <> 0"),
        0
    );
    check_sync_schema(&conn);
    let before = (table_counts(&conn), settings(&conn));
    drop(conn);

    // Voice profile (v6+): its key re-wrapped by the rotation and still opens.
    let store = common::reopen(dir, &keys);
    // A checkpoint (v8+) still opens after the rotation.
    if v >= 8 {
        let got = store.pass_checkpoints(&m1(), CKPT_STAMP).unwrap();
        assert_eq!(got[CKPT_PART], CKPT_DATA);
    } else {
        assert!(
            store
                .pass_checkpoints(&m1(), CKPT_STAMP)
                .unwrap()
                .is_empty()
        );
    }
    if v >= 6 {
        let profile = gid(8, 1);
        assert_eq!(
            store.voice_consent_clip(&profile).unwrap().as_deref(),
            Some(b"clip".as_slice())
        );
    }
    if v >= 7 {
        let m = store.get_meeting(&m1()).unwrap();
        assert_eq!(m.source_app.as_deref(), Some("zoom"));
        let folders = store.folders().unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(m.folder_gid.as_deref(), Some(folders[0].gid.as_str()));
        let tags = store.meeting_tags(&[m1()]).unwrap();
        assert_eq!(tags[&m1()].len(), 2);
        assert_eq!(
            store.calendar_info(&m1()).unwrap().unwrap()["title"],
            "Kế hoạch Q4"
        );
        assert_eq!(store.track_speakers(&m1()).unwrap().len(), 1);
        let segs = store.segments(&m1()).unwrap();
        assert!(segs[1].overlap && !segs[0].overlap);
    } else {
        assert!(store.folders().unwrap().is_empty());
        assert!(store.tags().unwrap().is_empty());
        let m = store.get_meeting(&m1()).unwrap();
        assert_eq!((m.source_app, m.folder_gid), (None, None));
        assert!(store.calendar_info(&m1()).unwrap().is_none());
        assert!(store.track_speakers(&m1()).unwrap().is_empty());
    }
    drop(store);

    // --- a second open changes nothing.
    let ring_before = common::ring(&keys);
    let conn = db::open(&db_path, &ring_before.db_key()).unwrap();
    let again = (table_counts(&conn), settings(&conn));
    drop(conn);
    assert_eq!(before, again, "re-opening the upgraded store wrote rows");
    assert_eq!(
        common::ring(&keys),
        ring_before,
        "no rotation on a clean open"
    );
    assert!(
        migrate::list_snapshots(&dir.join("snapshots"))
            .unwrap()
            .is_empty()
    );
    let conn = db::open(&db_path, &ring_before.db_key()).unwrap();
    assert_eq!(db::user_version(&conn).unwrap(), latest);
    assert_sound(&conn);
    drop(conn);

    // --- embeddings and index_gen after the upgrade.
    let store = common::reopen(dir, &keys);
    assert_eq!(store.index_gen(&m1()).unwrap(), 0);
    let needing = |s: &Store| s.meetings_needing_embeddings(MODEL, 10).unwrap();
    if v >= 5 {
        let chunks = store.embeddings(&m1(), MODEL).unwrap();
        assert_eq!(
            chunks,
            (0..2)
                .map(|c| EmbeddingChunk {
                    chunk: c,
                    t0_ms: i64::from(c) * 60_000,
                    t1_ms: i64::from(c + 1) * 60_000,
                    vec: vec_of(c),
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(store.all_embeddings(MODEL).unwrap().len(), 2);
        assert!(
            needing(&store).is_empty(),
            "v5+ vectors are current for generation 0"
        );
    } else {
        assert!(store.embeddings(&m1(), MODEL).unwrap().is_empty());
        assert_eq!(needing(&store), [m1()], "no vectors yet: needs indexing");
    }
    let read_gen = store.index_gen(&m1()).unwrap();
    store
        .update_segment_text(&segs[0].gid, "Chốt kế hoạch quý bốn (đã sửa)")
        .unwrap();
    assert_eq!(store.index_gen(&m1()).unwrap(), read_gen + 1);
    assert_eq!(needing(&store), [m1()], "an edit makes the vectors stale");
    let chunks = || {
        vec![EmbeddingChunk {
            chunk: 0,
            t0_ms: 0,
            t1_ms: 60_000,
            vec: vec_of(9),
        }]
    };
    assert!(matches!(
        store.put_embeddings(&m1(), MODEL, 1, read_gen, chunks()),
        Err(StoreError::IndexStale)
    ));
    store
        .put_embeddings(&m1(), MODEL, 1, read_gen + 1, chunks())
        .unwrap();
    assert!(needing(&store).is_empty());
    assert_eq!(store.embeddings(&m1(), MODEL).unwrap(), chunks());
    // The edit is searchable (the FTS row of the old text was replaced).
    assert!(
        store
            .search(&SearchQuery::new("da sua"))
            .unwrap()
            .iter()
            .any(|h| h.item_gid == segs[0].gid)
    );
}

/// The syncable tables and the `kind` their log rows use.
const SYNC_KINDS: [(&str, &str); 12] = [
    ("folders", "folder"),
    ("persons", "person"),
    ("tags", "tag"),
    ("meetings", "meeting"),
    ("tracks", "track"),
    ("speakers", "speaker"),
    ("segments", "segment"),
    ("notes_blocks", "note"),
    ("action_items", "action_item"),
    ("marks", "mark"),
    ("meeting_tags", "meeting_tag"),
    ("voice_profiles", "voice_profile"),
];

/// Schema 0009 (sync) after an upgrade: new tables are empty, ids are
/// set, notes are numbered in their old order, every live row and tombstone is
/// in `sync_log`, and the triggers keep it current.
fn check_sync_schema(conn: &Connection) {
    let counts = table_counts(conn);
    for t in [
        "devices",
        "leases",
        "peer_meetings",
        "conflict_copies",
        "synced_settings",
        "sync_pending",
    ] {
        assert_eq!(counts[t], 0, "{t}");
    }
    // Feed and device ids: quoted UUIDs, written once (a v9 fixture has them from its build).
    let uuid_setting = |key: &str| -> String {
        let json: String = conn
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .unwrap();
        let id: String = serde_json::from_str(&json).unwrap();
        assert!(uuid::Uuid::parse_str(&id).is_ok(), "{key}: {id}");
        id
    };
    assert_ne!(
        uuid_setting("sync.feed_id"),
        uuid_setting("sync.device_gid")
    );
    // Notes and action items keep their order: 0009 numbers them by id.
    for t in ["notes_blocks", "action_items"] {
        assert_eq!(
            scalar(
                conn,
                &format!("SELECT count(*) FROM {t} WHERE ord = '0001'")
            ),
            1,
            "{t}"
        );
    }
    // New columns have their defaults.
    assert_eq!(
        scalar(
            conn,
            "SELECT count(*) FROM meetings WHERE origin IS NOT NULL OR audio_origin IS NOT NULL
             OR base_lamport IS NOT NULL OR transcript_epoch <> 0 OR ai_epoch <> 0"
        ),
        0
    );
    assert_eq!(
        scalar(conn, "SELECT count(*) FROM segments WHERE epoch <> 0"),
        0
    );
    assert_eq!(
        scalar(
            conn,
            "SELECT count(*) FROM tracks WHERE lamport <> 0 OR cut_pages IS NOT NULL"
        ),
        0
    );
    assert_eq!(
        scalar(
            conn,
            "SELECT count(*) FROM tombstones WHERE cause IS NOT NULL OR origin IS NOT NULL"
        ),
        0
    );
    // sync_log: every live row and every tombstone has an entry of its kind.
    for (table, kind) in SYNC_KINDS {
        assert_eq!(
            scalar(
                conn,
                &format!(
                    "SELECT count(*) FROM {table} t WHERE NOT EXISTS
                     (SELECT 1 FROM sync_log l WHERE l.gid = t.gid AND l.kind = '{kind}')"
                )
            ),
            0,
            "{table} rows missing from sync_log"
        );
    }
    assert_eq!(
        scalar(
            conn,
            "SELECT count(*) FROM tombstones t WHERE NOT EXISTS
             (SELECT 1 FROM sync_log l WHERE l.gid = t.gid AND l.kind = 'tombstone')"
        ),
        0
    );
    // The Lamport clock moved up to the remote value (see `observe_lamport`).
    assert_eq!(
        scalar(
            conn,
            "SELECT CAST(value_json AS INTEGER) FROM settings WHERE key = 'lamport'"
        ),
        10_000
    );

    // Triggers: a write logs again (a new, higher seq), a tombstone replaces
    // the row's entry. Rolled back, so the caller's snapshot is unchanged.
    let tx = conn.unchecked_transaction().unwrap();
    let mark = common::gid(6, 1);
    let seq = |g: &str| -> (i64, String) {
        tx.query_row("SELECT seq, kind FROM sync_log WHERE gid = ?1", [g], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap()
    };
    let (seq0, kind0) = seq(&mark);
    assert_eq!(kind0, "mark");
    tx.execute("UPDATE marks SET t_ms = t_ms + 1 WHERE gid = ?1", [&mark])
        .unwrap();
    let (seq1, kind1) = seq(&mark);
    assert!(seq1 > seq0 && kind1 == "mark");
    tx.execute(
        "INSERT INTO tombstones (gid, kind, lamport, deleted_at, cause) VALUES (?1, 'mark', 99, 0, 'user')",
        [&mark],
    )
    .unwrap();
    let (seq2, kind2) = seq(&mark);
    assert!(seq2 > seq1 && kind2 == "tombstone");
    // A later write to the (not yet removed) row doesn't hide the tombstone.
    tx.execute("UPDATE marks SET t_ms = t_ms + 1 WHERE gid = ?1", [&mark])
        .unwrap();
    assert_eq!(seq(&mark), (seq2, "tombstone".to_string()));
    assert_eq!(
        scalar(
            &tx,
            &format!("SELECT count(*) FROM sync_log WHERE gid = '{mark}'")
        ),
        1
    );
    tx.rollback().unwrap();
}

macro_rules! upgrade_tests {
    ($($name:ident: $v:expr,)*) => {$(
        #[test]
        fn $name() {
            upgrade_and_check($v);
        }
    )*};
}

upgrade_tests! {
    upgrades_from_v1: 1,
    upgrades_from_v2: 2,
    upgrades_from_v3: 3,
    upgrades_from_v4: 4,
    upgrades_from_v5: 5,
    upgrades_from_v6: 6,
    upgrades_from_v7: 7,
    upgrades_from_v8: 8,
    upgrades_from_v9: 9,
}
