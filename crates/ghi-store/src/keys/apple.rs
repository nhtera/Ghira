// SPDX-License-Identifier: Apache-2.0
//! macOS / iOS Keychain (Security.framework) key store.
//!
//! The master key is a generic-password item identified by `(service,
//! account)`, non-synchronizable (never leaves the device via iCloud
//! Keychain).
//!
//! - `Protection { app_lock: true }`: the data-protection keychain with a
//!   `SecAccessControl` of `userPresence` (Touch ID / Face ID / password) and
//!   `WhenPasscodeSetThisDeviceOnly`. On macOS this needs a signed app with
//!   the keychain entitlements; an unsigned process gets
//!   `errSecMissingEntitlement` (-34018), reported as a clear
//!   [`StoreError::Keystore`]. This path is verified in the signed app
//!   (phase 12), not by the unit tests.
//!
//! Two constructors:
//! - [`KeychainStore::new`]: the CLI, dev builds, and the macOS alpha app.
//!   Login keychain on macOS (no entitlements needed); the data-protection
//!   keychain only with `app_lock`.
//! - [`KeychainStore::data_protection`]: iOS, and the macOS app once it ships
//!   with `keychain-access-groups` (that needs a provisioning profile with
//!   the Developer ID; an owner decision, see SECURITY.md). Always the
//!   data-protection keychain, `WhenUnlockedThisDeviceOnly`, with user
//!   presence when `app_lock` is set.
//!
//! A cancelled or denied prompt (or no UI available) is
//! [`StoreError::KeyLocked`].
//!
//! Saves are crash-safe: two slots (accounts `<account>` and `<account>.b`),
//! each item `generation (u64 LE) ‖ ring`. A save writes the slot that does
//! NOT hold the newest item (delete + add, so a changed access control
//! applies), then removes every other item. It never deletes the newest copy
//! before the new one exists, so a failure or crash at any step leaves a
//! loadable ring; [`KeyStore::load`] takes the highest generation across both
//! slots and both keychains.
//!
//! Invariant: one live `KeychainStore` per `(service, account)` per process.
//! Each instance caches the newest `(generation, slot)` so saves never read
//! the item (a Touch ID prompt with app lock); a second instance with a stale
//! cache could overwrite the slot holding the only ring. Share one instance
//! (an `Arc`) for everything that touches a store's key.

use security_framework::access_control::{ProtectionMode, SecAccessControl};
use security_framework::base::Error as SfError;
use security_framework::passwords::{
    PasswordOptions, delete_generic_password_options, generic_password,
    set_generic_password_options,
};
use security_framework::passwords_options::AccessControlOptions;

use zeroize::Zeroizing;

use super::{KeyRing, KeyStore, Protection};
use crate::StoreError;

const ERR_USER_CANCELED: i32 = -128;
const ERR_AUTH_FAILED: i32 = -25293;
const ERR_INTERACTION_NOT_ALLOWED: i32 = -25308;
const ERR_ITEM_NOT_FOUND: i32 = -25300;
const ERR_MISSING_ENTITLEMENT: i32 = -34018;

/// Master key in the Keychain.
pub struct KeychainStore {
    service: String,
    account: String,
    /// Always use the data-protection keychain.
    data_protection: bool,
    /// `(generation, slot)` of the newest item, from the last load or save:
    /// saves then need no read (which would prompt when app lock is on).
    newest: std::sync::Mutex<Option<(u64, usize)>>,
}

/// Which keychain an item lives in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flavor {
    /// The default: the login keychain on macOS.
    Legacy,
    /// The data-protection keychain (iOS always; macOS with entitlements).
    Protected,
}

impl KeychainStore {
    /// For unsigned binaries (CLI, dev, tests): the login keychain on macOS.
    pub fn new(service: &str, account: &str) -> KeychainStore {
        KeychainStore {
            service: service.to_owned(),
            account: account.to_owned(),
            data_protection: false,
            newest: Default::default(),
        }
    }

    /// For the signed app: always the data-protection keychain,
    /// `WhenUnlockedThisDeviceOnly`, with or without app lock. Needs the
    /// app's keychain entitlements (else a clear [`StoreError::Keystore`]).
    pub fn data_protection(service: &str, account: &str) -> KeychainStore {
        KeychainStore {
            service: service.to_owned(),
            account: account.to_owned(),
            data_protection: true,
            newest: Default::default(),
        }
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Option<(u64, usize)>> {
        self.newest.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn slots(&self) -> [String; 2] {
        [self.account.clone(), format!("{}.b", self.account)]
    }

    /// The keychains this store may have items in. On iOS both queries match
    /// the same items, so only one is used.
    fn flavors(&self) -> &'static [Flavor] {
        if self.data_protection || cfg!(target_os = "ios") {
            &[Flavor::Protected]
        } else {
            &[Flavor::Legacy, Flavor::Protected]
        }
    }

    /// Every item, as `(generation, slot index, flavor, ring)`. An item that
    /// doesn't parse is skipped (a newer, valid one may exist).
    fn items(&self) -> Result<Vec<(u64, usize, Flavor, KeyRing)>, StoreError> {
        let mut out = Vec::new();
        for (slot, account) in self.slots().iter().enumerate() {
            for &flavor in self.flavors() {
                let bytes = match self.read(flavor, account) {
                    Ok(Some(v)) => Zeroizing::new(v),
                    Ok(None) => continue,
                    // An unsigned macOS process cannot have stored anything there.
                    Err(StoreError::Keystore { detail })
                        if flavor == Flavor::Protected && detail.contains("-34018") =>
                    {
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                let Some((generation, ring)) = bytes.split_first_chunk::<8>() else {
                    continue;
                };
                if let Ok(ring) = KeyRing::from_bytes(ring) {
                    out.push((u64::from_le_bytes(*generation), slot, flavor, ring));
                }
            }
        }
        Ok(out)
    }

    fn query(&self, flavor: Flavor, account: &str) -> PasswordOptions {
        let mut o = PasswordOptions::new_generic_password(&self.service, account);
        o.set_access_synchronized(Some(false));
        if flavor == Flavor::Protected {
            o.use_protected_keychain();
        }
        o
    }

    fn flavor_for(&self, protection: Protection) -> Flavor {
        if self.data_protection || protection.app_lock || cfg!(target_os = "ios") {
            Flavor::Protected
        } else {
            Flavor::Legacy
        }
    }

    /// Adds a new item holding `bytes` under `account` (the caller removed
    /// any previous one).
    fn add(
        &self,
        flavor: Flavor,
        account: &str,
        protection: Protection,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        let mut o = self.query(flavor, account);
        if flavor == Flavor::Protected {
            let flags = if protection.app_lock {
                AccessControlOptions::USER_PRESENCE.bits()
            } else {
                0
            };
            let ac = SecAccessControl::create_with_protection(
                Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
                flags,
            )
            .map_err(map)?;
            o.set_access_control(ac);
        }
        set_generic_password_options(bytes, o).map_err(map)
    }

    fn read(&self, flavor: Flavor, account: &str) -> Result<Option<Vec<u8>>, StoreError> {
        match generic_password(self.query(flavor, account)) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.code() == ERR_ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(map(e)),
        }
    }

    fn remove(&self, flavor: Flavor, account: &str) -> Result<(), StoreError> {
        match delete_generic_password_options(self.query(flavor, account)) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ERR_ITEM_NOT_FOUND => Ok(()),
            // No entitlement means nothing can be stored there either.
            Err(e) if flavor == Flavor::Protected && e.code() == ERR_MISSING_ENTITLEMENT => Ok(()),
            Err(e) => Err(map(e)),
        }
    }
}

fn map(e: SfError) -> StoreError {
    match e.code() {
        ERR_USER_CANCELED | ERR_AUTH_FAILED | ERR_INTERACTION_NOT_ALLOWED => StoreError::KeyLocked,
        ERR_MISSING_ENTITLEMENT => StoreError::Keystore {
            detail: "the data-protection keychain needs a signed app with keychain entitlements \
                     (status -34018)"
                .into(),
        },
        code => StoreError::Keystore {
            detail: format!(
                "{} (status {code})",
                e.message()
                    .unwrap_or_else(|| "Security.framework error".into())
            ),
        },
    }
}

impl KeyStore for KeychainStore {
    fn load(&self) -> Result<Option<KeyRing>, StoreError> {
        let newest = self.items()?.into_iter().max_by_key(|(g, ..)| *g);
        *self.cache() = newest.as_ref().map(|(g, slot, ..)| (*g, *slot));
        Ok(newest.map(|(.., ring)| ring))
    }

    fn save(&self, ring: &KeyRing, protection: Protection) -> Result<(), StoreError> {
        let flavor = self.flavor_for(protection);
        let slots = self.slots();
        let cached = *self.cache();
        let newest = match cached {
            Some(n) => Some(n),
            None => self
                .items()?
                .into_iter()
                .max_by_key(|(g, ..)| *g)
                .map(|(g, slot, ..)| (g, slot)),
        };
        let generation = newest.map_or(0, |(g, _)| g) + 1;
        // Write the slot that doesn't hold the newest ring.
        let target = newest.map_or(0, |(_, slot)| 1 - slot);
        let mut bytes = Zeroizing::new(generation.to_le_bytes().to_vec());
        bytes.extend_from_slice(&ring.to_bytes());
        for &f in self.flavors() {
            self.remove(f, &slots[target])?;
        }
        self.add(flavor, &slots[target], protection, &bytes)?;
        // The new ring is stored: drop the older one (it may hold a wrap
        // secret a rotation just destroyed).
        *self.cache() = Some((generation, target));
        for &f in self.flavors() {
            self.remove(f, &slots[1 - target])?;
        }
        Ok(())
    }

    fn delete(&self) -> Result<(), StoreError> {
        *self.cache() = None;
        for account in self.slots() {
            for &flavor in self.flavors() {
                self.remove(flavor, &account)?;
            }
        }
        Ok(())
    }
}
