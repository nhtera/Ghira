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
