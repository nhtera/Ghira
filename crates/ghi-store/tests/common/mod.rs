// SPDX-License-Identifier: Apache-2.0
//! Shared helpers for the integration tests. Not every test uses everything.
#![allow(dead_code)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use ghi_store::StoreError;
use ghi_store::keys::{KeyRing, KeyStore, Protection};
use ghi_store::migrate::Migration;
use ghi_store::rowcrypt::Dek;
use ghi_store::store::{NewMeeting, NewSegment, Store};

/// An in-memory key store (the file-based one exists only in debug builds,
/// and the bench also runs in release).
#[derive(Default)]
pub struct MemKeyStore(Mutex<Option<KeyRing>>);

impl KeyStore for MemKeyStore {
    fn load(&self) -> Result<Option<KeyRing>, StoreError> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, ring: &KeyRing, _: Protection) -> Result<(), StoreError> {
        *self.0.lock().unwrap() = Some(ring.clone());
        Ok(())
    }
    fn delete(&self) -> Result<(), StoreError> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

pub type Keys = Arc<MemKeyStore>;

/// An empty key store (the first open creates the ring).
pub fn keys() -> Keys {
    Arc::new(MemKeyStore::default())
}

/// A key store that already holds `ring`.
pub fn keys_with(ring: KeyRing) -> Keys {
    Arc::new(MemKeyStore(Mutex::new(Some(ring))))
}

/// The ring currently in the key store.
pub fn ring(keys: &Keys) -> KeyRing {
    keys.load().unwrap().expect("a ring in the key store")
}

pub fn db_key(keys: &Keys) -> Dek {
    ring(keys).db_key()
}

pub fn open(dir: &Path) -> (Store, Keys) {
    let keys = keys();
    let store = Store::open(dir, keys.clone(), Protection::default()).expect("open store");
    (store, keys)
}

pub fn reopen(dir: &Path, keys: &Keys) -> Store {
    Store::open(dir, keys.clone(), Protection::default()).expect("reopen store")
}

pub fn open_with(dir: &Path, keys: &Keys, migrations: &[Migration]) -> Result<Store, StoreError> {
    Store::open_with_migrations(dir, keys.clone(), Protection::default(), migrations)
}

pub fn meeting(store: &Store, title: &str) -> String {
    store
        .create_meeting(NewMeeting {
            title: title.into(),
            started_at: 1_700_000_000_000,
            ..Default::default()
        })
        .expect("create meeting")
        .gid
}

pub fn seg(t0_ms: i64, t1_ms: i64, text: &str) -> NewSegment {
    NewSegment {
        t0_ms,
        t1_ms,
        text: text.into(),
        ..Default::default()
    }
}

/// Recursively copies a directory (the "attacker snapshot").
pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

// ------------------------------------------------- per-version fixtures
//
// `tests/fixtures/schema/vN.db` is a small SQLCipher database as schema
// version N left it (see `tests/schema_upgrade.rs` for what is in it and how
// to rebuild them). They are encrypted with a TEST key ring, never a real one.

use ghi_store::rowcrypt::{open_text, row_aad};

/// Newest schema version with a fixture.
pub const FIXTURE_VERSIONS: std::ops::RangeInclusive<u32> = 1..=8;

/// Gid `kind`/`i` of the fixtures (valid UUIDs, so `check_gid` accepts them).
pub fn gid(kind: u8, i: u16) -> String {
    format!("018f0000-0000-7000-8000-{kind:02x}{i:010x}")
}

/// The meeting every fixture keeps (title, lines, notes, ...).
pub fn m1() -> String {
    gid(1, 1)
}
/// A meeting whose delete had finished: only its tombstone is left.
pub fn m2() -> String {
    gid(1, 2)
}
/// A meeting whose key was shredded but whose rows are still there (a delete
/// interrupted by a crash); the next open finishes it.
pub fn m3() -> String {
    gid(1, 3)
}

/// The transcript lines of [`m1`], by time.
pub const M1_LINES: [&str; 3] = [
    "Chốt kế hoạch quý bốn",
    "Đồng ý ngân sách cho dự án Hà Nội",
    "Anh Bình sẽ gửi báo cáo",
];

/// The fixtures' key ring. Fixed bytes, test-only.
pub fn fixture_ring() -> KeyRing {
    let mut b = b"GHKR".to_vec();
    b.extend([1u8, 0]);
    b.extend([0x11u8; 32]);
    b.extend([0x22u8; 32]);
    KeyRing::from_bytes(&b).expect("fixture ring")
}

pub fn fixture_file(version: u32) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/schema")
        .join(format!("v{version}.db"))
}

/// Copies fixture `version` to `<dir>/ghira.db`; returns a key store holding
/// the fixture ring.
pub fn install_fixture(version: u32, dir: &Path) -> Keys {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::copy(fixture_file(version), dir.join("ghira.db")).unwrap_or_else(|e| {
        panic!("fixture v{version}: {e} (rebuild: see tests/fixtures/README.md)")
    });
    keys_with(fixture_ring())
}

/// The plaintext of [`m1`]'s transcript lines read straight from `conn`
/// (unwrap the meeting key with `ring`, decrypt each sealed line): works on
/// any schema version, without the `Store` API.
pub fn raw_m1_lines(conn: &rusqlite::Connection, ring: &KeyRing) -> Vec<String> {
    let m1 = m1();
    let wrapped: Vec<u8> = conn
        .query_row(
            "SELECT dek_wrapped FROM meetings WHERE gid = ?1",
            [&m1],
            |r| r.get(0),
        )
        .unwrap();
    let dek = ring
        .unwrap_dek(&wrapped, &m1)
        .expect("unwrap the meeting key");
    let mut stmt = conn
        .prepare(
            "SELECT gid, text_ct FROM segments
             WHERE meeting_id = (SELECT id FROM meetings WHERE gid = ?1) ORDER BY t0_ms",
        )
        .unwrap();
    let rows: Vec<(String, Vec<u8>)> = stmt
        .query_map([&m1], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    rows.into_iter()
        .map(|(gid, ct)| open_text(&dek, &ct, &row_aad("segments", "text_ct", &gid)).unwrap())
        .collect()
}
