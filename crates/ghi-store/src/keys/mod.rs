// SPDX-License-Identifier: Apache-2.0
//! The master key and where it lives.
//!
//! One 256-bit master key per device, kept in the OS keystore as a
//! device-only, non-synchronizable item ([`KeyStore`]):
//! - macOS/iOS: Keychain via Security.framework ([`apple`]); with app lock on,
//!   bound to user presence (Touch ID / Face ID / password).
//! - Windows: DPAPI-protected file in the non-roaming app data dir ([`windows`]).
//! - Android: Keystore (phase 17; [`android`] reports it as unsupported).
//! - Debug builds only: a plain file ([`dev`]). Release builds do not contain it.
//!
//! What the keystore holds is a [`KeyRing`]:
//! - the **master key**: HKDF-SHA256 derives the SQLCipher key from it;
//! - the **wrap secret**: with the master key it derives the key that wraps
//!   each meeting's [`Dek`]. It is **rotated on every meeting delete**: the
//!   remaining DEKs are re-wrapped and the old secret is destroyed, so a copy
//!   of the database taken *before* the delete (an APFS local snapshot, a
//!   stray backup) can no longer unwrap the deleted meeting's key;
//! - during a rotation, the previous wrap secret (crash safety);
//! - when a recovery phrase is set, the phrase-derived key, so the recovery
//!   file can be rewritten after each rotation.

use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::StoreError;
use crate::rowcrypt::{self, Dek};

#[cfg(target_os = "android")]
pub mod android;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub mod apple;
#[cfg(debug_assertions)]
pub mod dev;
#[cfg(windows)]
pub mod windows;

/// HKDF info labels. Changing one makes existing data unreadable.
const DB_KEY_INFO: &[u8] = b"ghira/sqlcipher/v1";
const WRAP_KEY_INFO: &[u8] = b"ghira/dek-wrap/v2";
/// Serialized key ring: magic, version, flags, then 32-byte keys.
const RING_MAGIC: &[u8; 4] = b"GHKR";
const RING_V1: u8 = 1;
const FLAG_PREV: u8 = 1;
const FLAG_RECOVERY: u8 = 2;

#[derive(Clone, Zeroize, ZeroizeOnDrop, PartialEq, Eq)]
pub struct MasterKey([u8; 32]);

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(..)")
    }
}

impl MasterKey {
    pub fn generate() -> MasterKey {
        let mut k = [0u8; 32];
        OsRng.fill_bytes(&mut k);
        MasterKey(k)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> MasterKey {
        MasterKey(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    fn derive(&self, info: &[u8]) -> Dek {
        let mut out = [0u8; 32];
        Hkdf::<Sha256>::new(None, &self.0)
            .expand(info, &mut out)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        Dek::from_bytes(out)
    }

    /// The SQLCipher database key (raw 256-bit key, no PBKDF).
    pub fn db_key(&self) -> Dek {
        self.derive(DB_KEY_INFO)
    }
}

/// A 256-bit secret, zeroed on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop, PartialEq, Eq)]
struct Secret([u8; 32]);

impl Secret {
    fn generate() -> Secret {
        let mut k = [0u8; 32];
        OsRng.fill_bytes(&mut k);
        Secret(k)
    }
}

/// Everything the OS keystore holds for one data directory (see the module docs).
#[derive(Clone, PartialEq, Eq)]
pub struct KeyRing {
    master: MasterKey,
    wrap: Secret,
    prev_wrap: Option<Secret>,
    recovery: Option<Secret>,
}

impl std::fmt::Debug for KeyRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyRing")
            .field("rotating", &self.prev_wrap.is_some())
            .field("recovery", &self.recovery.is_some())
            .finish_non_exhaustive()
    }
}

impl KeyRing {
    pub fn generate() -> KeyRing {
        KeyRing {
            master: MasterKey::generate(),
            wrap: Secret::generate(),
            prev_wrap: None,
            recovery: None,
        }
    }

    pub fn master(&self) -> &MasterKey {
        &self.master
    }

    /// The SQLCipher database key.
    pub fn db_key(&self) -> Dek {
        self.master.db_key()
    }

    fn wrap_key(&self, secret: &Secret) -> Dek {
        let mut ikm = Zeroizing::new([0u8; 64]);
        ikm[..32].copy_from_slice(self.master.as_bytes());
        ikm[32..].copy_from_slice(&secret.0);
        let mut out = [0u8; 32];
        Hkdf::<Sha256>::new(None, ikm.as_slice())
            .expand(WRAP_KEY_INFO, &mut out)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        Dek::from_bytes(out)
    }

    /// Wraps a meeting DEK for `meetings.dek_wrapped` with the current wrap
    /// secret, bound to the meeting gid.
    pub fn wrap_dek(&self, dek: &Dek, meeting_gid: &str) -> Vec<u8> {
        rowcrypt::seal(
            &self.wrap_key(&self.wrap),
            dek.as_bytes(),
            &wrap_aad(meeting_gid),
        )
    }

    /// Unwraps with the current secret, or during a rotation the previous one.
    pub fn unwrap_dek(&self, wrapped: &[u8], meeting_gid: &str) -> Result<Dek, StoreError> {
        match self.unwrap_with(&self.wrap, wrapped, meeting_gid) {
            Err(StoreError::Decrypt) if self.prev_wrap.is_some() => {
                self.unwrap_with(self.prev_wrap.as_ref().unwrap(), wrapped, meeting_gid)
            }
            r => r,
        }
    }

    fn unwrap_with(
        &self,
        secret: &Secret,
        wrapped: &[u8],
        meeting_gid: &str,
    ) -> Result<Dek, StoreError> {
        let bytes = Zeroizing::new(rowcrypt::open(
            &self.wrap_key(secret),
            wrapped,
            &wrap_aad(meeting_gid),
        )?);
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| StoreError::Decrypt)?;
        Ok(Dek::from_bytes(arr))
    }

    /// Starts a rotation: a new current wrap secret, the old one kept as
    /// previous. Save the ring, re-wrap every DEK with [`Self::wrap_dek`], then
    /// [`Self::finish_rotation`] and save again. A crash in between is
    /// finished on the next open ([`Self::is_rotating`]).
    pub fn begin_rotation(&mut self) {
        let old = std::mem::replace(&mut self.wrap, Secret::generate());
        self.prev_wrap = Some(old);
    }

    /// Drops the previous wrap secret: DEKs wrapped with it (deleted meetings,
    /// old copies of the database) can no longer be unwrapped.
    pub fn finish_rotation(&mut self) {
        self.prev_wrap = None;
    }

    pub fn is_rotating(&self) -> bool {
        self.prev_wrap.is_some()
    }

    /// The phrase-derived key that wraps `recovery.bin`, if a phrase is set.
    pub fn recovery_key(&self) -> Option<&[u8; 32]> {
        self.recovery.as_ref().map(|s| &s.0)
    }

    pub fn set_recovery_key(&mut self, key: Option<[u8; 32]>) {
        self.recovery = key.map(Secret);
    }

    /// Serialized for the keystore: `GHKR | v1 | flags | master | wrap | [prev] | [recovery]`.
    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        let mut flags = 0;
        if self.prev_wrap.is_some() {
            flags |= FLAG_PREV;
        }
        if self.recovery.is_some() {
            flags |= FLAG_RECOVERY;
        }
        let mut out = Zeroizing::new(Vec::with_capacity(6 + 4 * 32));
        out.extend_from_slice(RING_MAGIC);
        out.push(RING_V1);
        out.push(flags);
        out.extend_from_slice(self.master.as_bytes());
        out.extend_from_slice(&self.wrap.0);
        for s in [&self.prev_wrap, &self.recovery].into_iter().flatten() {
            out.extend_from_slice(&s.0);
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<KeyRing, StoreError> {
        let bad = || StoreError::Invalid("unreadable key ring in the keystore".into());
        let rest = bytes.strip_prefix(RING_MAGIC.as_slice()).ok_or_else(bad)?;
        let [RING_V1, flags, keys @ ..] = rest else {
            return Err(bad());
        };
        let n = 2 + usize::from(flags & FLAG_PREV != 0) + usize::from(flags & FLAG_RECOVERY != 0);
        if flags & !(FLAG_PREV | FLAG_RECOVERY) != 0 || keys.len() != n * 32 {
            return Err(bad());
        }
        let mut it = keys.chunks_exact(32).map(|c| Secret(c.try_into().unwrap()));
        let master = MasterKey(it.next().unwrap().0);
        let wrap = it.next().unwrap();
        let prev_wrap = (flags & FLAG_PREV != 0).then(|| it.next().unwrap());
        let recovery = (flags & FLAG_RECOVERY != 0).then(|| it.next().unwrap());
        Ok(KeyRing {
            master,
            wrap,
            prev_wrap,
            recovery,
        })
    }
}

fn wrap_aad(meeting_gid: &str) -> Vec<u8> {
    format!("dek:{meeting_gid}").into_bytes()
}

/// How the stored master key is protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Protection {
    /// App lock: reading the key needs user presence (biometrics or the
    /// device password) where the platform supports it.
    pub app_lock: bool,
}

/// A device-only home for the [`KeyRing`]. Backends store
/// [`KeyRing::to_bytes`] as an opaque secret.
pub trait KeyStore: Send + Sync {
    /// The stored ring, or `None` if there is none yet.
    fn load(&self) -> Result<Option<KeyRing>, StoreError>;
    /// Stores (or replaces) the ring.
    fn save(&self, ring: &KeyRing, protection: Protection) -> Result<(), StoreError>;
    /// Removes the ring. Without a recovery phrase, the data is gone for good.
    fn delete(&self) -> Result<(), StoreError>;
}

/// Loads the key ring, creating and saving one on first run.
pub fn load_or_create(store: &dyn KeyStore, protection: Protection) -> Result<KeyRing, StoreError> {
    if let Some(k) = store.load()? {
        return Ok(k);
    }
    let k = KeyRing::generate();
    store.save(&k, protection)?;
    Ok(k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivations_are_stable_and_distinct() {
        let m = MasterKey::from_bytes([7; 32]);
        assert_eq!(m.db_key(), m.db_key());
        assert_ne!(m.db_key(), MasterKey::from_bytes([8; 32]).db_key());
        let r = KeyRing::generate();
        assert_ne!(r.db_key(), r.wrap_key(&r.wrap));
    }

    #[test]
    fn dek_wrapping_is_bound_to_the_meeting() {
        let r = KeyRing::generate();
        let dek = Dek::generate();
        let w = r.wrap_dek(&dek, "m1");
        assert_eq!(r.unwrap_dek(&w, "m1").unwrap(), dek);
        assert!(r.unwrap_dek(&w, "m2").is_err());
        assert!(KeyRing::generate().unwrap_dek(&w, "m1").is_err());
    }

    #[test]
    fn rotation_retires_old_wraps() {
        let mut r = KeyRing::generate();
        let (keep, gone) = (Dek::generate(), Dek::generate());
        let (w_keep, w_gone) = (r.wrap_dek(&keep, "keep"), r.wrap_dek(&gone, "gone"));
        let db_key = r.db_key();
        r.begin_rotation();
        // Mid-rotation both old and new wraps open (crash safety).
        assert_eq!(r.unwrap_dek(&w_keep, "keep").unwrap(), keep);
        let w_keep2 = r.wrap_dek(&keep, "keep");
        r.finish_rotation();
        assert_eq!(r.unwrap_dek(&w_keep2, "keep").unwrap(), keep);
        // An old copy of the database is now useless for every meeting.
        assert!(r.unwrap_dek(&w_gone, "gone").is_err());
        assert!(r.unwrap_dek(&w_keep, "keep").is_err());
        // The database key does not change.
        assert_eq!(r.db_key(), db_key);
    }

    #[test]
    fn ring_serialization() {
        let mut r = KeyRing::generate();
        assert_eq!(KeyRing::from_bytes(&r.to_bytes()).unwrap(), r);
        r.begin_rotation();
        r.set_recovery_key(Some([9; 32]));
        let back = KeyRing::from_bytes(&r.to_bytes()).unwrap();
        assert_eq!(back, r);
        assert!(back.is_rotating());
        assert_eq!(back.recovery_key(), Some(&[9; 32]));
        let bytes = r.to_bytes();
        assert!(KeyRing::from_bytes(&bytes[..bytes.len() - 1]).is_err());
        assert!(KeyRing::from_bytes(&[0; 70]).is_err());
    }
}
