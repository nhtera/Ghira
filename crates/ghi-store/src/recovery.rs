// SPDX-License-Identifier: Apache-2.0
//! The optional 24-word recovery key.
//!
//! A [`RecoveryPhrase`] is a BIP-39 English mnemonic of 256 bits of entropy.
//! It wraps the master key (`recovery.bin` in the data dir): HKDF-SHA256 of
//! the entropy gives a key that seals the master key with
//! [`rowcrypt::seal`]. Losing both the device keystore and the phrase means
//! the data is gone; with the phrase, [`unwrap_master`] restores access.
//!
//! The phrase is the only copy of the entropy the user holds, so it is
//! shown once and never stored by the app.

use bip39::{Language, Mnemonic};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::keys::KeyRing;
use crate::rowcrypt::{self, Dek};
use crate::{Result, StoreError};

const WORD_COUNT: usize = 24;
const WRAP_INFO: &[u8] = b"ghira/recovery-wrap/v1";
const WRAP_AAD: &[u8] = b"ghira/recovery/v2";
/// `recovery.bin` = magic ‖ version ‖ sealed key ring.
const MAGIC: &[u8; 4] = b"GHIR";
const VERSION: u8 = 1;

/// A 24-word recovery phrase (256-bit entropy). Zeroed on drop.
#[derive(Zeroize, ZeroizeOnDrop, PartialEq, Eq)]
pub struct RecoveryPhrase {
    entropy: [u8; 32],
}

impl std::fmt::Debug for RecoveryPhrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoveryPhrase(..)")
    }
}

impl RecoveryPhrase {
    /// A fresh random phrase.
    pub fn generate() -> RecoveryPhrase {
        let mut entropy = [0u8; 32];
        OsRng.fill_bytes(&mut entropy);
        RecoveryPhrase { entropy }
    }

    /// Parses what the user typed: any case, any whitespace between words.
    /// Fails on the wrong word count, an unknown word or a bad checksum.
    pub fn parse(input: &str) -> Result<RecoveryPhrase> {
        let mut words: Vec<String> = input.split_whitespace().map(str::to_lowercase).collect();
        let normalized: Zeroizing<String> = Zeroizing::new(words.join(" "));
        words.iter_mut().for_each(Zeroize::zeroize);
        if normalized.split(' ').count() != WORD_COUNT {
            return Err(StoreError::Invalid("a recovery phrase has 24 words".into()));
        }
        let m = Mnemonic::parse_in_normalized(Language::English, &normalized).map_err(|_| {
            StoreError::Invalid("recovery phrase is not valid (check the words)".into())
        })?;
        let mut bytes = Zeroizing::new(m.to_entropy());
        let entropy: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| StoreError::Invalid("a recovery phrase has 24 words".into()))?;
        bytes.zeroize();
        Ok(RecoveryPhrase { entropy })
    }

    /// The 24 words, in order.
    pub fn words(&self) -> Vec<&'static str> {
        Mnemonic::from_entropy_in(Language::English, &self.entropy)
            .expect("32 bytes is valid BIP-39 entropy")
            .words()
            .collect()
    }

    /// The key derived from the phrase (HKDF-SHA256 of the entropy). The
    /// store keeps it in the [`KeyRing`] so it can rewrite `recovery.bin`
    /// after each rotation with [`wrap_ring_with_key`].
    pub fn wrap_key(&self) -> Zeroizing<[u8; 32]> {
        let mut out = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(None, &self.entropy)
            .expand(WRAP_INFO, out.as_mut())
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        out
    }
}

/// Seals the whole key ring under the phrase. The blob is the content of
/// `recovery.bin`.
pub fn wrap_ring(ring: &KeyRing, phrase: &RecoveryPhrase) -> Vec<u8> {
    wrap_ring_with_key(ring, &phrase.wrap_key())
}

/// [`wrap_ring`] with the key from [`RecoveryPhrase::wrap_key`].
pub fn wrap_ring_with_key(ring: &KeyRing, key: &[u8; 32]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.push(VERSION);
    out.extend_from_slice(&rowcrypt::seal(
        &Dek::from_bytes(*key),
        &ring.to_bytes(),
        WRAP_AAD,
    ));
    out
}

/// Recovers the key ring from a [`wrap_ring`] blob. A wrong phrase or a
/// damaged blob gives [`StoreError::Decrypt`].
pub fn unwrap_ring(blob: &[u8], phrase: &RecoveryPhrase) -> Result<KeyRing> {
    let sealed = blob
        .strip_prefix(MAGIC.as_slice())
        .and_then(|b| b.strip_prefix(&[VERSION]))
        .ok_or(StoreError::Decrypt)?;
    let bytes = Zeroizing::new(rowcrypt::open(
        &Dek::from_bytes(*phrase.wrap_key()),
        sealed,
        WRAP_AAD,
    )?);
    KeyRing::from_bytes(&bytes).map_err(|_| StoreError::Decrypt)
}
