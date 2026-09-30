// SPDX-License-Identifier: Apache-2.0
//! Per-meeting data keys (DEKs) and row encryption.
//!
//! Each meeting has its own random 256-bit DEK, stored only wrapped by the
//! master key (`meetings.dek_wrapped`). It encrypts the meeting's audio pages
//! ([`crate::bundle`]) and its text columns (`segments.text_ct`,
//! `notes_blocks.body_ct`, `action_items.text_ct`, `meetings.title_ct`).
//! Destroying the wrapped DEK makes all of it unreadable (crypto-shred).
//!
//! The DEK itself never encrypts: HKDF derives one subkey for text rows
//! ([`ROWS_INFO`]) and one for audio pages ([`AUDIO_INFO`], used by
//! [`crate::bundle`]), so the random-nonce and counter-nonce schemes never
//! share a key.
//!
//! Sealed values use XChaCha20-Poly1305 with a random 24-byte nonce:
//! `version (1 byte) || nonce || aead(plaintext)`. The AAD binds a value to
//! its place (table, column, row gid) so ciphertexts can't be swapped.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::StoreError;

/// Format version, the first byte of every sealed value (enables re-wrapping
/// or algorithm changes later).
pub const FORMAT_V1: u8 = 1;
/// HKDF labels of the DEK subkeys. Changing one makes data unreadable.
pub const ROWS_INFO: &[u8] = b"ghira/rows/v1";
pub const AUDIO_INFO: &[u8] = b"ghira/audio/v1";

/// Nonce length of XChaCha20-Poly1305.
pub const NONCE_LEN: usize = 24;
/// Authentication tag length.
pub const TAG_LEN: usize = 16;

/// A 256-bit symmetric key, zeroed on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop, PartialEq, Eq)]
pub struct Dek([u8; 32]);

impl std::fmt::Debug for Dek {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Dek(..)")
    }
}

impl Dek {
    pub fn generate() -> Dek {
        let mut k = [0u8; 32];
        OsRng.fill_bytes(&mut k);
        Dek(k)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Dek {
        Dek(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// A subkey for one purpose (HKDF-SHA256, `info` = label).
    pub fn subkey(&self, info: &[u8]) -> Dek {
        let mut out = [0u8; 32];
        Hkdf::<Sha256>::new(None, &self.0)
            .expand(info, &mut out)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        Dek(out)
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&self.0).into())
    }
}

/// Encrypts `plaintext` bound to `aad` with `key` as is (callers pass a
/// purpose subkey). Output: `version || nonce || ciphertext || tag`.
pub fn seal(key: &Dek, plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let ct = key
        .cipher()
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("XChaCha20-Poly1305 encryption cannot fail for in-memory input");
    let mut out = Vec::with_capacity(1 + NONCE_LEN + ct.len());
    out.push(FORMAT_V1);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

/// Decrypts a value from [`seal`]; fails if the key, AAD or data is wrong.
pub fn open(key: &Dek, sealed: &[u8], aad: &[u8]) -> Result<Vec<u8>, StoreError> {
    let Some((&FORMAT_V1, rest)) = sealed.split_first() else {
        return Err(StoreError::Decrypt);
    };
    if rest.len() < NONCE_LEN + TAG_LEN {
        return Err(StoreError::Decrypt);
    }
    let (nonce, ct) = rest.split_at(NONCE_LEN);
    key.cipher()
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad })
        .map_err(|_| StoreError::Decrypt)
}

/// Encrypts a text column value with the meeting DEK's rows subkey. `aad`
/// is [`row_aad`].
pub fn seal_text(dek: &Dek, text: &str, aad: &[u8]) -> Vec<u8> {
    seal(&dek.subkey(ROWS_INFO), text.as_bytes(), aad)
}

pub fn open_text(dek: &Dek, sealed: &[u8], aad: &[u8]) -> Result<String, StoreError> {
    let plain = Zeroizing::new(open(&dek.subkey(ROWS_INFO), sealed, aad)?);
    String::from_utf8(plain.to_vec()).map_err(|_| StoreError::Decrypt)
}

/// AAD for an encrypted column: `table.column:gid`.
pub fn row_aad(table: &str, column: &str, gid: &str) -> Vec<u8> {
    format!("{table}.{column}:{gid}").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_binding() {
        let k = Dek::generate();
        let aad = row_aad("segments", "text_ct", "g1");
        let ct = seal_text(&k, "Chốt kế hoạch quý bốn", &aad);
        assert_eq!(open_text(&k, &ct, &aad).unwrap(), "Chốt kế hoạch quý bốn");
        // Wrong key, wrong row, tampered data: all rejected.
        assert!(open_text(&Dek::generate(), &ct, &aad).is_err());
        assert!(open_text(&k, &ct, &row_aad("segments", "text_ct", "g2")).is_err());
        let mut bad = ct.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(open_text(&k, &bad, &aad).is_err());
        assert!(open_text(&k, &ct[..10], &aad).is_err());
        // Unknown format version.
        let mut v2 = ct.clone();
        v2[0] = 2;
        assert!(open_text(&k, &v2, &aad).is_err());
        // Rows are sealed with a subkey, not the DEK itself.
        assert!(open(&k, &ct, &aad).is_err());
        assert_ne!(k.subkey(ROWS_INFO), k.subkey(AUDIO_INFO));
        // Fresh nonce every time.
        assert_ne!(seal_text(&k, "x", &aad), seal_text(&k, "x", &aad));
    }
}
