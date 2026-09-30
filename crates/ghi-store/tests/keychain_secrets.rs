// SPDX-License-Identifier: Apache-2.0
//! Live Keychain test (macOS) for provider API keys: throwaway items under
//! `com.nhtera.ghira.test`, always deleted. Skips when the login keychain is
//! not usable (CI without a login session).
#![cfg(target_os = "macos")]

use ghi_store::StoreError;
use ghi_store::keys::secrets::{KeychainSecrets, SecretStore};

const SERVICE: &str = "com.nhtera.ghira.test";

/// Deletes the accounts when dropped, pass or fail.
struct Cleanup(Vec<String>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let store = KeychainSecrets::new(SERVICE);
        for account in &self.0 {
            let _ = store.delete(account);
        }
    }
}

fn unavailable(e: &StoreError) -> bool {
    matches!(e, StoreError::KeyLocked)
        || matches!(e, StoreError::Keystore { detail }
            if ["-25307", "-25291"].iter().any(|c| detail.contains(c)))
}

#[test]
fn set_get_replace_delete() {
    let tag = format!("{}-{}", std::process::id(), ghi_store::new_gid());
    let (a, b) = (format!("test-a-{tag}"), format!("test-b-{tag}"));
    let _cleanup = Cleanup(vec![a.clone(), b.clone()]);
    let store = KeychainSecrets::new(SERVICE);

    assert!(store.get(&a).map(|v| v.is_none()).unwrap_or(true));
    match store.set(&a, b"test-key-not-real") {
        Ok(()) => {}
        Err(e) if unavailable(&e) => {
            assert!(
                std::env::var_os("CI").is_none(),
                "keychain unavailable on CI: {e}"
            );
            eprintln!("skipping: login keychain unavailable ({e})");
            return;
        }
        Err(e) => panic!("set failed: {e}"),
    }
    assert_eq!(
        store.get(&a).unwrap().unwrap().as_slice(),
        b"test-key-not-real"
    );

    // Replaced in place, not duplicated.
    store.set(&a, b"test-key-2-not-real").unwrap();
    assert_eq!(
        store.get(&a).unwrap().unwrap().as_slice(),
        b"test-key-2-not-real"
    );

    // Another account is another item.
    assert!(store.get(&b).unwrap().is_none());
    store.set(&b, b"other").unwrap();
    assert_eq!(store.get(&b).unwrap().unwrap().as_slice(), b"other");

    store.delete(&a).unwrap();
    store.delete(&a).unwrap();
    assert!(store.get(&a).unwrap().is_none());
    assert_eq!(store.get(&b).unwrap().unwrap().as_slice(), b"other");
    store.delete(&b).unwrap();

    assert!(store.set("bad/name", b"x").is_err());
}
