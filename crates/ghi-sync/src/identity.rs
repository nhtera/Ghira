// SPDX-License-Identifier: Apache-2.0
//! This device's sync identity (doc 07 §3.1, decision D10): an X25519 static
//! keypair and a `device_gid`, kept in the device-only [`SecretStore`] under
//! the account [`IDENTITY_ACCOUNT`] and created when sync is first enabled.
//!
//! The secret newtypes print `<redacted>` and zeroize on drop.

use std::fmt;

use ghi_store::keys::secrets::SecretStore;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{Result, SyncError, not_yet};

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

    /// Reads the identity from the secret store, creating it if there is none.
    pub fn load_or_create(_secrets: &dyn SecretStore, _device_gid: &str) -> Result<Self> {
        not_yet("identity::Identity::load_or_create")
    }

    /// Destroys the identity ("Delete everything").
    pub fn delete(_secrets: &dyn SecretStore) -> Result<()> {
        not_yet("identity::Identity::delete")
    }
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
}
