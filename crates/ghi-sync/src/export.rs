// SPDX-License-Identifier: Apache-2.0
//! "Export for another device" (doc 07 §10; slice 15-M): the fallback when
//! devices can't reach each other. It writes the existing GHIX archive
//! (passphrase, Argon2id + XChaCha20-Poly1305 STREAM) with entries `rec`
//! (wire records, DEKs inside the encrypted archive), `tomb` and `bundle`
//! (verbatim audio). Import goes through the same merge engine with
//! `origin = file`: tombstoned gids are refused, keys re-wrapped, ciphertexts
//! verified. No pairing is needed.

use std::path::Path;

use crate::store::SyncStore;
use crate::{Result, not_yet};

/// What an export wrote or an import applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExportReport {
    pub meetings: usize,
    pub tombstones: usize,
    pub audio_bytes: u64,
}

/// Writes an archive of the given meetings (all if empty) to `path`.
pub fn export_for_device(
    _store: &dyn SyncStore,
    _meeting_gids: &[String],
    _path: &Path,
    _passphrase: &str,
) -> Result<ExportReport> {
    not_yet("export::export_for_device")
}

/// Merges an archive into the store. Nothing changes on a wrong passphrase.
pub fn import_from_archive(
    _store: &dyn SyncStore,
    _path: &Path,
    _passphrase: &str,
) -> Result<ExportReport> {
    not_yet("export::import_from_archive")
}
