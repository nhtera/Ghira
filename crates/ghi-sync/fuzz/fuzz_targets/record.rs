// SPDX-License-Identifier: Apache-2.0
//! Typed sync records (CBOR) through the real merge engine: arbitrary bytes
//! decoded as a record and applied to a store that keeps its state between
//! runs. Never a panic in decoding or in `apply_rows`; a record applied twice
//! does not fail the second time when it succeeded the first.
#![no_main]

use std::sync::{Mutex, OnceLock};

use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::Store;
use ghi_store::sync::devices::{DeviceRole, NewDevice};
use ghi_store::sync::records::Record;
use libfuzzer_sys::fuzz_target;

const PEER: &str = "01a10000-0000-7000-8000-000000000001";

struct Rig {
    store: Store,
    _dir: tempfile::TempDir,
}

fn rig() -> &'static Mutex<Rig> {
    static RIG: OnceLock<Mutex<Rig>> = OnceLock::new();
    RIG.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(
            dir.path(),
            std::sync::Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        store
            .pin_device(
                &NewDevice {
                    gid: PEER.into(),
                    name: "fuzz peer".into(),
                    platform: "ios".into(),
                    role: DeviceRole::Spoke,
                    static_pub: [5; 32],
                },
                &[6; 32],
            )
            .unwrap();
        Mutex::new(Rig { store, _dir: dir })
    })
}

fuzz_target!(|data: &[u8]| {
    let Ok(record) = ciborium::from_reader::<Record, _>(data) else {
        return;
    };
    // The accessors every caller uses.
    let _ = (
        record.kind(),
        record.gid(),
        record.version(),
        record.meeting_gid(),
    );
    let rig = rig().lock().unwrap();
    let rows = [record];
    if rig.store.apply_rows(PEER, &rows).is_ok() {
        assert!(
            rig.store.apply_rows(PEER, &rows).is_ok(),
            "a record that applied once failed when it came again"
        );
    }
});
