// SPDX-License-Identifier: Apache-2.0
//! The `vocabulary.packs` synced setting: only known pack ids sync (red-team M8).

mod common;

use ghi_store::StoreError;
use ghi_store::sync::records::{Record, SettingRec, Version};
use ghi_store::sync::settings::{PACK_IDS, SYNCED_KEYS, is_synced_key, setting_gid};

fn rec(value: &str, lamport: i64) -> SettingRec {
    SettingRec {
        gid: setting_gid("vocabulary.packs"),
        version: Version {
            lamport,
            origin: "peer".into(),
        },
        key: "vocabulary.packs".into(),
        value_json: Some(value.into()),
        ..Default::default()
    }
}

#[test]
fn packs_setting_is_on_the_allowlist() {
    assert!(is_synced_key("vocabulary.packs"));
    assert!(SYNCED_KEYS.contains(&"vocabulary.packs"));
    assert_eq!(PACK_IDS.len(), 8);
}

#[test]
fn only_known_pack_ids_are_accepted_locally() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    for bad in [
        "\"medical-en\"",
        "[\"medical\"]",
        "[\"medical-fr\"]",
        "[1]",
        "[\"../etc\"]",
        "{}",
        "not json",
    ] {
        assert!(
            matches!(
                store.put_synced("vocabulary.packs", bad),
                Err(StoreError::Invalid(_))
            ),
            "{bad}"
        );
    }
    store.put_synced("vocabulary.packs", "[]").unwrap();
    store
        .put_synced("vocabulary.packs", "[\"medical-vi\", \"tech-en\"]")
        .unwrap();
    let feed = store.changes_since(0, 256).unwrap();
    let sent = feed
        .changes
        .iter()
        .find_map(|c| match &c.record {
            Record::Setting(s) if s.key == "vocabulary.packs" => Some(s.clone()),
            _ => None,
        })
        .expect("the setting is in the feed");
    assert_eq!(
        sent.value_json.as_deref(),
        Some("[\"medical-vi\",\"tech-en\"]")
    );
}

#[test]
fn a_peers_packs_are_validated_then_written_as_the_setting() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    assert!(store.apply_synced(&rec("[\"klingon-en\"]", 5)).is_err());
    assert!(store.get_setting("vocabulary.packs").unwrap().is_none());

    store
        .apply_synced(&rec("[\"legal-en\",\"finance-vi\"]", 6))
        .unwrap();
    assert_eq!(
        store.get_setting("vocabulary.packs").unwrap().unwrap(),
        serde_json::json!(["legal-en", "finance-vi"])
    );
    // An older value does not win.
    store.apply_synced(&rec("[]", 3)).unwrap();
    assert_eq!(
        store.get_setting("vocabulary.packs").unwrap().unwrap(),
        serde_json::json!(["legal-en", "finance-vi"])
    );
}
