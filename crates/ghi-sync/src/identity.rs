// SPDX-License-Identifier: Apache-2.0
//! This device's sync identity (doc 07 §3.1, decision D10): an X25519 static
//! keypair and a `device_gid`, kept in the device-only [`SecretStore`] under
//! the account [`IDENTITY_ACCOUNT`] and created when sync is first enabled.
//!
//! The secret newtypes print `<redacted>` and zeroize on drop.

use std::fmt;

use ghi_store::keys::secrets::SecretStore;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{Result, SyncError};

/// The [`SecretStore`] account of the identity.
pub const IDENTITY_ACCOUNT: &str = "ghira.sync.identity";

/// A 32-byte pre-shared key (the QR PSK or a pair's long-term PSK).
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct Psk([u8; 32]);

impl Psk {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// 32 fresh random bytes from the OS.
    pub fn random() -> Result<Self> {
        use rand_core::{OsRng, RngCore};
        let mut b = [0u8; 32];
        OsRng
            .try_fill_bytes(&mut b)
            .map_err(|e| SyncError::Noise(format!("no randomness: {e}")))?;
        Ok(Self(b))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for Psk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Psk(<redacted>)")
    }
}

/// An X25519 private key. Not `Clone`: it exists in as few places as possible.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct StaticSecret([u8; 32]);

impl StaticSecret {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for StaticSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StaticSecret(<redacted>)")
    }
}

/// The device identity: who we are to a peer.
#[derive(Debug)]
pub struct Identity {
    /// UUIDv7 text, the same value as `settings['sync.device_gid']`.
    pub device_gid: String,
    pub secret: StaticSecret,
    pub public: [u8; 32],
}

impl Identity {
    /// A new keypair (Noise `25519`) under a new gid.
    pub fn generate() -> Result<Self> {
        let params = crate::transport::noise_params()?;
        let kp = snow::Builder::new(params)
            .generate_keypair()
            .map_err(|e| SyncError::Noise(e.to_string()))?;
        let (Ok(private), Ok(public)) = (
            <[u8; 32]>::try_from(kp.private.as_slice()),
            <[u8; 32]>::try_from(kp.public.as_slice()),
        ) else {
            return Err(SyncError::Noise("unexpected key length".into()));
        };
        Ok(Self {
            device_gid: ghi_store::new_gid(),
            secret: StaticSecret(private),
            public,
        })
    }

    /// The identity in the secret store, if one was created.
    pub fn load(secrets: &dyn SecretStore) -> Result<Option<Self>> {
        let Some(blob) = secrets.get(IDENTITY_ACCOUNT)? else {
            return Ok(None);
        };
        // version 1: 0x01 || x25519 secret (32) || device_gid (16, UUID bytes)
        let ok = blob.len() == 1 + 32 + 16 && blob[0] == BLOB_VERSION;
        if !ok {
            return Err(SyncError::Wire(
                "the stored sync identity is unreadable".into(),
            ));
        }
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&blob[1..33]);
        let mut gid = [0u8; 16];
        gid.copy_from_slice(&blob[33..49]);
        let secret = StaticSecret(secret);
        let public = public_of(&secret)?;
        Ok(Some(Self {
            device_gid: uuid::Uuid::from_bytes(gid).to_string(),
            secret,
            public,
        }))
    }

    /// Reads the identity from the secret store, creating it if there is none.
    /// `device_gid` is the gid to create it under (the one in the settings;
    /// empty = a new one); an existing identity keeps its own.
    pub fn load_or_create(secrets: &dyn SecretStore, device_gid: &str) -> Result<Self> {
        if let Some(id) = Self::load(secrets)? {
            return Ok(id);
        }
        let mut id = Self::generate()?;
        if !device_gid.is_empty() {
            id.device_gid = uuid::Uuid::parse_str(device_gid)
                .map_err(|_| SyncError::Wire("the device gid is not a UUID".into()))?
                .to_string();
        }
        let gid = uuid::Uuid::parse_str(&id.device_gid)
            .map_err(|_| SyncError::Wire("the device gid is not a UUID".into()))?;
        let mut blob = Zeroizing::new(Vec::with_capacity(49));
        blob.push(BLOB_VERSION);
        blob.extend_from_slice(id.secret.as_bytes());
        blob.extend_from_slice(gid.as_bytes());
        secrets.set(IDENTITY_ACCOUNT, &blob)?;
        Ok(id)
    }

    /// Destroys the identity ("Delete everything"). No error if there is none.
    pub fn delete(secrets: &dyn SecretStore) -> Result<()> {
        secrets.delete(IDENTITY_ACCOUNT)?;
        Ok(())
    }
}

const BLOB_VERSION: u8 = 1;

/// The X25519 public key of `secret`.
fn public_of(secret: &StaticSecret) -> Result<[u8; 32]> {
    use snow::params::DHChoice;
    use snow::resolvers::{CryptoResolver, DefaultResolver};
    let mut dh = DefaultResolver
        .resolve_dh(&DHChoice::Curve25519)
        .ok_or_else(|| SyncError::Noise("no X25519".into()))?;
    dh.set(secret.as_bytes());
    <[u8; 32]>::try_from(dh.pubkey()).map_err(|_| SyncError::Noise("unexpected key length".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_never_print() {
        let psk = Psk::from_bytes([7; 32]);
        let secret = StaticSecret::from_bytes([9; 32]);
        for s in [format!("{psk:?}"), format!("{secret:?}")] {
            assert!(s.contains("<redacted>"), "{s}");
            assert!(!s.contains('7') && !s.contains('9'), "{s}");
        }
        let id = Identity::generate().unwrap();
        let shown = format!("{id:?}");
        assert!(shown.contains("<redacted>"));
        assert!(!shown.contains(&format!("{:?}", id.secret.as_bytes())));
    }

    #[test]
    fn generated_identities_differ() {
        let (a, b) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        assert_ne!(a.public, b.public);
        assert_ne!(a.device_gid, b.device_gid);
        assert_ne!(Psk::random().unwrap(), Psk::random().unwrap());
    }

    #[test]
    fn psk_is_zeroed_by_zeroize() {
        let mut psk = Psk::from_bytes([5; 32]);
        psk.zeroize();
        assert_eq!(psk.as_bytes(), &[0; 32]);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn create_load_destroy_with_the_file_store() {
        use ghi_store::keys::secrets::FileSecrets;
        let dir = tempfile::tempdir().unwrap();
        let secrets = FileSecrets::new(dir.path());
        assert!(Identity::load(&secrets).unwrap().is_none());

        let gid = ghi_store::new_gid();
        let made = Identity::load_or_create(&secrets, &gid).unwrap();
        assert_eq!(made.device_gid, gid);
        let again = Identity::load_or_create(&secrets, "").unwrap();
        assert_eq!(again.device_gid, gid);
        assert_eq!(again.public, made.public);
        assert_eq!(again.secret.as_bytes(), made.secret.as_bytes());
        // Another gid does not replace an existing identity.
        let other = Identity::load_or_create(&secrets, &ghi_store::new_gid()).unwrap();
        assert_eq!(other.device_gid, gid);

        Identity::delete(&secrets).unwrap();
        assert!(Identity::load(&secrets).unwrap().is_none());
        Identity::delete(&secrets).unwrap();
        let fresh = Identity::load_or_create(&secrets, "").unwrap();
        assert_ne!(fresh.public, made.public);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn a_bad_gid_or_a_corrupt_blob_is_an_error_not_a_new_identity() {
        use ghi_store::keys::secrets::FileSecrets;
        let dir = tempfile::tempdir().unwrap();
        let secrets = FileSecrets::new(dir.path());
        assert!(Identity::load_or_create(&secrets, "not-a-uuid").is_err());
        assert!(Identity::load(&secrets).unwrap().is_none());
        secrets.set(IDENTITY_ACCOUNT, b"junk").unwrap();
        assert!(Identity::load_or_create(&secrets, "").is_err());
    }

    #[test]
    fn the_public_key_matches_what_noise_made() {
        let id = Identity::generate().unwrap();
        assert_eq!(public_of(&id.secret).unwrap(), id.public);
    }
}
