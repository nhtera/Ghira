// SPDX-License-Identifier: Apache-2.0
//! Review L8: "use this" on a copy whose target is gone fails with
//! `NotFound` and keeps the copy, instead of losing the chosen text.

mod common;

use ghi_store::StoreError;
use ghi_store::rowcrypt::{Dek, row_aad, seal_text};
use ghi_store::sync::devices::{DeviceRole, NewDevice};
use ghi_store::sync::records::{Bytes, ConflictCopyRec, Record, Version};

#[test]
fn using_a_copy_whose_target_is_gone_is_not_found_and_keeps_the_copy() {
    let (hub_dir, spoke_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (hub, hub_keys) = common::open(hub_dir.path());
    let (spoke, _) = common::open(spoke_dir.path());
    let spoke_gid = spoke.sync_device_gid().unwrap();
    hub.pin_device(
        &NewDevice {
            gid: spoke_gid.clone(),
            name: "Peer".into(),
            platform: "ios".into(),
            role: DeviceRole::Spoke,
            static_pub: [2; 32],
        },
        &[1; 32],
    )
    .unwrap();
    let m = common::meeting(&spoke, "Họp");
    let Record::Meeting(mrec) = spoke.encode_record("meeting", &m, true).unwrap().unwrap() else {
        unreachable!()
    };
    let dek = Dek::from_bytes(mrec.dek.clone().unwrap().0.try_into().unwrap());
    hub.apply_rows(&spoke_gid, &[Record::Meeting(mrec)])
        .unwrap();
    let cg = common::gid(13, 1);
    let copy = Record::ConflictCopy(ConflictCopyRec {
        gid: cg.clone(),
        version: Version {
            lamport: 70,
            origin: spoke_gid.clone(),
        },
        meeting_gid: m.clone(),
        target_kind: "meeting".into(),
        target_gid: m.clone(),
        field: "title_ct".into(),
        value_ct: Some(Bytes(seal_text(
            &dek,
            "Tiêu đề thua",
            &row_aad("conflict_copies", "value_ct", &cg),
        ))),
        created_at: Some(1_700_000_000_000),
        ..Default::default()
    });
    hub.apply_rows(&spoke_gid, &[copy]).unwrap();
    assert_eq!(hub.conflict_copies(&m).unwrap().len(), 1);

    // The target is superseded: the copy now points at a row that is not there.
    let raw =
        ghi_store::db::open(&hub_dir.path().join("ghira.db"), &common::db_key(&hub_keys)).unwrap();
    raw.execute(
        "UPDATE conflict_copies SET target_gid = 'gone' WHERE gid = ?1",
        [&cg],
    )
    .unwrap();
    drop(raw);

    let err = hub.resolve_conflict(&cg, true).unwrap_err();
    assert!(matches!(err, StoreError::NotFound { .. }), "{err:?}");
    assert_eq!(hub.conflict_copies(&m).unwrap().len(), 1, "the copy stays");
    // Dismissing still works.
    hub.resolve_conflict(&cg, false).unwrap();
    assert!(hub.conflict_copies(&m).unwrap().is_empty());
}
