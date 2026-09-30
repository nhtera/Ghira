// SPDX-License-Identifier: Apache-2.0
//! 24-word recovery phrase: parsing, checksum, wrapping the master key.

use ghi_store::StoreError;
use ghi_store::keys::KeyRing;
use ghi_store::recovery::{RecoveryPhrase, unwrap_ring, wrap_ring, wrap_ring_with_key};

/// BIP-39 test vector: 32 bytes of 0xff.
const VECTOR: &str = "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote";

#[test]
fn generate_has_24_words_that_parse_back() {
    let p = RecoveryPhrase::generate();
    let words = p.words();
    assert_eq!(words.len(), 24);
    let again = RecoveryPhrase::parse(&words.join(" ")).unwrap();
    assert!(p == again);
    assert!(p != RecoveryPhrase::generate());
}

#[test]
fn parse_normalises_case_and_whitespace() {
    let a = RecoveryPhrase::parse(VECTOR).unwrap();
    let messy = format!("  {}\n\t", VECTOR.to_uppercase().replace(' ', " \n  "));
    assert!(a == RecoveryPhrase::parse(&messy).unwrap());
    assert_eq!(a.words().len(), 24);
    assert_eq!(a.words()[23], "vote");
}

#[test]
fn rejects_bad_phrases() {
    // Wrong last word: checksum fails.
    let bad_checksum = VECTOR.replace("vote", "zoo");
    let cases = [
        bad_checksum.as_str(),
        "",
        "zoo zoo zoo",
        // 12 valid words is a valid BIP-39 phrase, but not ours.
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        // Unknown word.
        "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo xyzzy",
        // 25 words.
        "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote vote",
    ];
    for c in cases {
        assert!(
            matches!(RecoveryPhrase::parse(c), Err(StoreError::Invalid(_))),
            "{c:?}"
        );
    }
}

#[test]
fn wraps_and_unwraps_the_master_key() {
    let master = KeyRing::generate();
    let phrase = RecoveryPhrase::generate();
    let blob = wrap_ring(&master, &phrase);
    assert_ne!(blob, wrap_ring(&master, &phrase), "fresh nonce every time");
    assert!(unwrap_ring(&blob, &phrase).unwrap() == master);

    // Through the words the user wrote down.
    let typed = RecoveryPhrase::parse(&phrase.words().join("  ").to_uppercase()).unwrap();
    assert!(unwrap_ring(&blob, &typed).unwrap() == master);

    // Wrong phrase, damaged blob.
    assert!(matches!(
        unwrap_ring(&blob, &RecoveryPhrase::generate()),
        Err(StoreError::Decrypt)
    ));
    let mut bad = blob.clone();
    *bad.last_mut().unwrap() ^= 1;
    assert!(matches!(
        unwrap_ring(&bad, &phrase),
        Err(StoreError::Decrypt)
    ));
    assert!(matches!(
        unwrap_ring(&blob[..20], &phrase),
        Err(StoreError::Decrypt)
    ));
}

#[test]
fn debug_does_not_leak() {
    let p = RecoveryPhrase::parse(VECTOR).unwrap();
    assert!(!format!("{p:?}").contains("zoo"));
}

#[test]
fn restored_key_gives_the_same_db_key() {
    let master = KeyRing::generate();
    let phrase = RecoveryPhrase::generate();
    let blob = wrap_ring(&master, &phrase);
    assert_eq!(&blob[..5], b"GHIR\x01");
    let restored = unwrap_ring(&blob, &phrase).unwrap();
    assert!(restored.db_key() == master.db_key());
    // Wrong magic or version is rejected, not misparsed.
    let mut bad = blob.clone();
    bad[4] = 2;
    assert!(matches!(
        unwrap_ring(&bad, &phrase),
        Err(StoreError::Decrypt)
    ));
    assert!(matches!(
        unwrap_ring(&blob[5..], &phrase),
        Err(StoreError::Decrypt)
    ));
}

#[test]
fn rotated_ring_is_rewritten_with_the_stored_recovery_key() {
    let phrase = RecoveryPhrase::generate();
    let mut ring = KeyRing::generate();
    ring.set_recovery_key(Some(*phrase.wrap_key()));
    ring.begin_rotation();
    ring.finish_rotation();
    let blob = wrap_ring_with_key(&ring, ring.recovery_key().unwrap());
    let back = unwrap_ring(&blob, &phrase).unwrap();
    assert!(back == ring);
    assert_eq!(&blob[..5], b"GHIR\x01");
}
