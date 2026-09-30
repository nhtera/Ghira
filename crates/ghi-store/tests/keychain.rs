// SPDX-License-Identifier: Apache-2.0
//! Live Keychain test (macOS): one throwaway item under
//! `com.nhtera.ghira.test`, always deleted. Skips when the login keychain is
//! not usable (CI without a login session). The app-lock path
//! (`SecAccessControl` + Touch ID) needs the signed app and is verified in
//! phase 12.
#![cfg(target_os = "macos")]

use ghi_store::StoreError;
use ghi_store::keys::apple::KeychainStore;
use ghi_store::keys::{KeyRing, KeyStore, Protection};

const SERVICE: &str = "com.nhtera.ghira.test";

/// Deletes the item when dropped, pass or fail.
struct Cleanup(KeychainStore);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = self.0.delete();
    }
}

fn unavailable(e: &StoreError) -> bool {
    // errSecInteractionNotAllowed maps to KeyLocked; also
    // errSecNoDefaultKeychain (-25307), errSecNotAvailable (-25291).
    matches!(e, StoreError::KeyLocked)
        || matches!(e, StoreError::Keystore { detail }
            if ["-25307", "-25291"].iter().any(|c| detail.contains(c)))
}

#[test]
fn save_load_replace_delete() {
    let account = format!("test-{}-{}", std::process::id(), ghi_store::new_gid());
    let store = KeychainStore::new(SERVICE, &account);
    let _cleanup = Cleanup(KeychainStore::new(SERVICE, &account));

    let k1 = KeyRing::generate();
    match store.save(&k1, Protection::default()) {
        Ok(()) => {}
        Err(e) if unavailable(&e) => {
            // Locally a locked or missing login keychain is an environment
            // problem; on CI it must not pass silently.
            assert!(
                std::env::var_os("CI").is_none(),
                "keychain unavailable on CI: {e}"
            );
            eprintln!("skipping: login keychain unavailable ({e})");
            return;
        }
        Err(e) => panic!("save failed: {e}"),
    }
    assert!(store.load().unwrap().unwrap() == k1);

    let k2 = KeyRing::generate();
    store.save(&k2, Protection::default()).unwrap();
    let loaded = store.load().unwrap().unwrap();
    assert!(loaded == k2 && loaded != k1);

    // A different account is a different item.
    let other = KeychainStore::new(SERVICE, &format!("{account}-other"));
    assert!(other.load().unwrap().is_none());

    store.delete().unwrap();
    assert!(store.load().unwrap().is_none());
    store.delete().unwrap(); // deleting nothing is fine
}

#[test]
fn app_lock_without_entitlements_fails_clearly() {
    let account = format!("test-lock-{}-{}", std::process::id(), ghi_store::new_gid());
    let store = KeychainStore::new(SERVICE, &account);
    let _cleanup = Cleanup(KeychainStore::new(SERVICE, &account));
    match store.save(&KeyRing::generate(), Protection { app_lock: true }) {
        // Signed with entitlements (or a runner that has them): it worked.
        Ok(()) => assert!(store.load().is_ok()),
        Err(StoreError::Keystore { detail }) => {
            eprintln!(
                "app_lock unavailable here, as expected for an unsigned test binary: {detail}"
            );
        }
        Err(StoreError::KeyLocked) => eprintln!("app_lock prompt was cancelled"),
        Err(e) => panic!("unexpected error: {e:?}"),
    }
}

#[test]
fn data_protection_constructor_needs_entitlements_and_fails_clearly() {
    let account = format!("test-dp-{}-{}", std::process::id(), ghi_store::new_gid());
    let store = KeychainStore::data_protection(SERVICE, &account);
    let _cleanup = Cleanup(KeychainStore::data_protection(SERVICE, &account));
    match store.save(&KeyRing::generate(), Protection::default()) {
        Ok(()) => assert!(store.load().unwrap().is_some()),
        Err(StoreError::Keystore { detail }) => assert!(detail.contains("-34018"), "{detail}"),
        Err(StoreError::KeyLocked) => {}
        Err(e) => panic!("unexpected error: {e:?}"),
    }
}

/// Items are `generation (u64 LE) ‖ ring` in two slots, `<account>` and
/// `<account>.b` (see `keys::apple`).
fn item(generation: u64, ring: &KeyRing) -> Vec<u8> {
    let mut v = generation.to_le_bytes().to_vec();
    v.extend_from_slice(&ring.to_bytes());
    v
}

#[test]
fn an_interrupted_save_never_loses_the_newest_ring() {
    use security_framework::passwords::{get_generic_password, set_generic_password};
    let account = format!("test-slots-{}-{}", std::process::id(), ghi_store::new_gid());
    let slot_b = format!("{account}.b");
    let store = KeychainStore::new(SERVICE, &account);
    let _cleanup = Cleanup(KeychainStore::new(SERVICE, &account));

    let k1 = KeyRing::generate();
    match store.save(&k1, Protection::default()) {
        Ok(()) => {}
        Err(e) if unavailable(&e) => {
            assert!(
                std::env::var_os("CI").is_none(),
                "keychain unavailable on CI: {e}"
            );
            eprintln!("skipping: login keychain unavailable ({e})");
            return;
        }
        Err(e) => panic!("save failed: {e}"),
    }
    let k2 = KeyRing::generate();
    store.save(&k2, Protection::default()).unwrap();
    // One item after a completed save.
    let items = [&account, &slot_b]
        .iter()
        .filter(|a| get_generic_password(SERVICE, a).is_ok())
        .count();
    assert_eq!(items, 1);

    // A save that died after adding the new item, before removing the old
    // one: both exist, and the newer generation wins (whichever slot).
    let k3 = KeyRing::generate();
    let (old_slot, new_slot) = if get_generic_password(SERVICE, &account).is_ok() {
        (&account, &slot_b)
    } else {
        (&slot_b, &account)
    };
    set_generic_password(SERVICE, new_slot, &item(1000, &k3)).unwrap();
    let fresh = KeychainStore::new(SERVICE, &account);
    assert!(fresh.load().unwrap().unwrap() == k3);
    // A stale lower generation never wins.
    set_generic_password(SERVICE, old_slot, &item(1, &k1)).unwrap();
    assert!(fresh.load().unwrap().unwrap() == k3);

    // The next save keeps the newest until the new one exists, then leaves
    // exactly one item.
    let k4 = KeyRing::generate();
    fresh.save(&k4, Protection::default()).unwrap();
    assert!(
        KeychainStore::new(SERVICE, &account)
            .load()
            .unwrap()
            .unwrap()
            == k4
    );
    let items = [&account, &slot_b]
        .iter()
        .filter(|a| get_generic_password(SERVICE, a).is_ok())
        .count();
    assert_eq!(items, 1);

    fresh.delete().unwrap();
    assert!(fresh.load().unwrap().is_none());
    assert!(get_generic_password(SERVICE, &slot_b).is_err());
}
