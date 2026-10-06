// SPDX-License-Identifier: Apache-2.0
//! "Export for another device" (doc 07 §10; slice 15-M): the fallback when
//! devices can't reach each other. It writes the existing GHIX archive
//! (passphrase, Argon2id + XChaCha20-Poly1305 STREAM) with these entries:
//!
//! ```text
//! manifest              CBOR {format, v, device}
//! tomb/<n>              CBOR [SyncTombstone]   (<= 1000 each)
//! rec/<n>               CBOR [Record]          (wire records, DEK inside the meeting's)
//! bundle/<meeting>/<track>   the track's bundle file, verbatim
//! ```
//!
//! The DEKs travel only inside the encrypted archive (the in-memory entries
//! never touch the disk in plaintext on either side).
//!
//! Import goes through the same merge engine as a session (`apply_tombs`,
//! then `apply_rows`, then audio through `RawImport`, so every page is
//! verified), applied as the fixed "Export file" device
//! ([`ghi_store::export::FILE_ORIGIN_GID`], state `known`: no key, never
//! connects). That sender is a *spoke* to the engine (rows from it are merged
//! by `(lamport, origin gid)` like a push to a hub, concurrent edits keep the
//! higher version and conflict copies are made as usual), and the versions
//! themselves carry the real writers' gids, so tie-breaks are the same
//! whichever way the data arrives. A fixed gid (not one per export) keeps a
//! second import of the same file idempotent. Tombstoned gids are refused by
//! the engine, keys are re-wrapped under this device's ring, and every
//! ciphertext is opened with its AAD before anything commits. No pairing is
//! needed.

use std::io;
use std::path::{Path, PathBuf};

use ghi_store::export::{self as archive, Entry, KdfParams};
use ghi_store::store::Store;
use ghi_store::sync::apply::ApplyOutcome;
use ghi_store::sync::records::{Bytes, Record, SyncTombstone};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::audio::OfferResult;
use crate::store::SyncStore;
use crate::wire::{self, BundleHeader, RefuseReason, TrackOffer};
use crate::{Result, SyncError};

/// Shortest passphrase (characters), as for "Export everything".
pub const MIN_PASSPHRASE_CHARS: usize = 8;

const FORMAT: &str = "ghi-sync-export";
const VERSION: u32 = 1;
const MANIFEST: &str = "manifest";
const TOMB_DIR: &str = "tomb/";
const REC_DIR: &str = "rec/";
const BUNDLE_DIR: &str = "bundle/";
/// One `rec/<n>` entry stays under this (in-memory entries are at most 1 MiB).
const REC_ENTRY_SOFT: usize = 384 * 1024;
const REC_ENTRY_MAX: usize = 1024 * 1024;
const REC_ENTRY_RECORDS: usize = 256;
const TOMB_ENTRY: usize = 1000;

/// What an export wrote or an import applied.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportReport {
    /// Meetings written, or on import taken (new, merged or already here).
    pub meetings: usize,
    /// Meetings the file holds that this device refused (tombstoned here).
    pub refused: usize,
    pub tombstones: usize,
    /// Audio tracks written, or on import complete here afterwards.
    pub tracks: usize,
    /// Bundle bytes written, or stored by this import.
    pub audio_bytes: u64,
    /// On import: the gids of the meetings taken (new, merged or already
    /// here). Empty for an export.
    pub taken: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: String,
    v: u32,
    /// The exporting device's gid (informational: the importer applies the
    /// rows as the "Export file" device and each version names its writer).
    device: String,
}

fn invalid(what: &str) -> SyncError {
    SyncError::Store(ghi_store::StoreError::Invalid(what.into()))
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    ciborium::into_writer(value, &mut out).map_err(|_| SyncError::Wire("unencodable".into()))?;
    Ok(out)
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8], what: &str) -> Result<T> {
    ciborium::from_reader(bytes).map_err(|_| invalid(&format!("the file's {what} is malformed")))
}

/// Writes an archive of `meetings` (every finished meeting if `None`) to
/// `out`, sealed with `passphrase` (at least [`MIN_PASSPHRASE_CHARS`]).
pub fn export_for_device(
    store: &Store,
    meetings: Option<&[String]>,
    passphrase: &str,
    out: &Path,
) -> Result<ExportReport> {
    export_for_device_with(store, meetings, passphrase, out, &KdfParams::default())
}

/// [`export_for_device`] with chosen key-derivation cost (tests lower it).
pub fn export_for_device_with(
    store: &Store,
    meetings: Option<&[String]>,
    passphrase: &str,
    out: &Path,
    kdf: &KdfParams,
) -> Result<ExportReport> {
    let passphrase = Zeroizing::new(passphrase.to_string());
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(invalid("the passphrase is too short"));
    }
    let gids = store.sync_export_meetings(meetings)?;
    if gids.is_empty() {
        return Err(invalid("there are no meetings to export"));
    }
    let mut entries = vec![Entry::bytes(
        MANIFEST,
        encode(&Manifest {
            format: FORMAT.into(),
            v: VERSION,
            device: store.sync_device_gid()?,
        })?,
    )];

    let tombs = store.sync_export_tombstones(meetings.is_none())?;
    for (i, chunk) in tombs.chunks(TOMB_ENTRY).enumerate() {
        entries.push(Entry::bytes(format!("{TOMB_DIR}{i:08}"), encode(&chunk)?));
    }

    let mut batch: Vec<Record> = Vec::new();
    let mut size = 0usize;
    let mut n = 0usize;
    let mut flush = |batch: &mut Vec<Record>, entries: &mut Vec<Entry>| -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let bytes = encode(&*batch)?;
        if bytes.len() > REC_ENTRY_MAX {
            return Err(invalid("a record is too large to export"));
        }
        entries.push(Entry::bytes(format!("{REC_DIR}{n:08}"), bytes));
        n += 1;
        batch.clear();
        Ok(())
    };
    for (kind, gid) in store.sync_export_rows(&gids)? {
        // A row deleted since the listing is simply not exported.
        let Some(rec) = store.encode_record(kind, &gid, true)? else {
            continue;
        };
        let len = encode(&rec)?.len();
        if !batch.is_empty() && (batch.len() >= REC_ENTRY_RECORDS || size + len > REC_ENTRY_SOFT) {
            flush(&mut batch, &mut entries)?;
            size = 0;
        }
        size += len;
        batch.push(rec);
    }
    flush(&mut batch, &mut entries)?;

    let tracks = store.sync_export_tracks(&gids)?;
    let mut audio_bytes = 0u64;
    for t in &tracks {
        audio_bytes += std::fs::metadata(&t.path)?.len();
        entries.push(Entry::file(
            format!("{BUNDLE_DIR}{}/{}", t.meeting_gid, t.track_gid),
            &t.path,
        ));
    }

    archive::export_entries_with(&entries, out, &passphrase, kdf)?;
    Ok(ExportReport {
        meetings: gids.len(),
        refused: 0,
        tombstones: tombs.len(),
        tracks: tracks.len(),
        audio_bytes,
        taken: Vec::new(),
    })
}

/// Removes the directory the archive's files are unpacked into.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Merges an archive into the store. Nothing changes on a wrong passphrase
/// or a damaged file (the whole archive is verified before the first apply).
/// Importing the same file again changes nothing.
pub fn import_from_device(store: &Store, path: &Path, passphrase: &str) -> Result<ExportReport> {
    let passphrase = Zeroizing::new(passphrase.to_string());
    // Inside the data directory (same volume as the bundles; leftovers of a
    // crash are cleaned when the store opens: `.ghi-import-` prefix).
    let scratch = Scratch(
        store
            .dir()
            .join(format!(".ghi-import-sync-{}", uuid::Uuid::new_v4())),
    );
    let imported = archive::import_archive_entries(path, &passphrase, &scratch.0)?;

    let manifest: Manifest = decode(
        imported
            .entry(MANIFEST)
            .ok_or_else(|| invalid("this file is not an export for another device"))?,
        "manifest",
    )?;
    if manifest.format != FORMAT || manifest.v != VERSION {
        return Err(invalid("this file is not an export for another device"));
    }

    let mut names: Vec<&String> = imported.names.iter().collect();
    names.sort();
    let from = store.sync_file_origin()?;
    let mut report = ExportReport::default();

    // Tombstones first and absorbing, then rows (doc 07 §7.5).
    for name in names.iter().filter(|n| n.starts_with(TOMB_DIR)) {
        let tombs: Vec<SyncTombstone> = decode(entry(&imported, name)?, "tombstones")?;
        report.tombstones += store.apply_tombs(&from, &tombs)?.applied.len();
    }
    for name in names.iter().filter(|n| n.starts_with(REC_DIR)) {
        let rows: Vec<Record> = decode(entry(&imported, name)?, "records")?;
        let applied = store.apply_rows(&from, &rows)?;
        for (rec, (_, outcome)) in rows.iter().zip(&applied.results) {
            if matches!(rec, Record::Meeting(_)) {
                match outcome {
                    ApplyOutcome::Tombstoned => report.refused += 1,
                    _ => {
                        report.meetings += 1;
                        report.taken.push(rec.gid().to_string());
                    }
                }
            }
        }
    }

    for name in names.iter().filter(|n| n.starts_with(BUNDLE_DIR)) {
        let mut parts = name[BUNDLE_DIR.len()..].split('/');
        let (Some(meeting_gid), Some(track_gid), None) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(invalid("the file's audio entry is malformed"));
        };
        let file = scratch.0.join(name.as_str());
        if let Some(bytes) = import_track(store, &from, meeting_gid, track_gid, &file)? {
            report.tracks += 1;
            report.audio_bytes += bytes;
        }
    }
    Ok(report)
}

fn entry<'a>(imported: &'a archive::Imported, name: &str) -> Result<&'a [u8]> {
    imported
        .entry(name)
        .ok_or_else(|| invalid("the file is malformed"))
}

/// Feeds one unpacked bundle through the verifying raw import. `Some(bytes)`
/// when the track is complete here afterwards; `None` when it is refused
/// (its meeting or track is tombstoned, or its meeting was not taken).
fn import_track(
    store: &Store,
    from: &str,
    meeting_gid: &str,
    track_gid: &str,
    file: &Path,
) -> Result<Option<u64>> {
    let header = ghi_store::bundle::raw_header(file)?;
    let bytes = std::fs::metadata(file)?.len();
    let offer = TrackOffer {
        track_gid: track_gid.to_string(),
        meeting_gid: meeting_gid.to_string(),
        header: BundleHeader {
            magic: Bytes(header[..4].to_vec()),
            version: header[4],
            prefix: Bytes(header[5..].to_vec()),
        },
        // The receiving store sizes by `bytes`; the page count is not used.
        pages: 0,
        bytes,
        complete: true,
    };
    let sync: &dyn SyncStore = store;
    let mut have = match sync.track_offer(from, &offer)? {
        OfferResult::Complete => return Ok(Some(bytes)),
        OfferResult::Have(n) => n,
        OfferResult::Refuse(RefuseReason::StorageFull) => {
            return Err(SyncError::Io(io::Error::new(
                io::ErrorKind::StorageFull,
                "not enough free space for the audio",
            )));
        }
        OfferResult::Refuse(_) => return Ok(None),
    };
    loop {
        let records = ghi_store::bundle::raw_records(
            file,
            u32::try_from(have).unwrap_or(u32::MAX),
            wire::MAX_TRACK_PAGES as u32,
        )?;
        if records.is_empty() {
            break;
        }
        have = sync.track_push(from, track_gid, &header[5..], have, &records)?;
    }
    // The final record completes the track; anything else is a cut-off file.
    match sync.track_offer(from, &offer)? {
        OfferResult::Complete => Ok(Some(bytes)),
        _ => Err(invalid("the file's audio is incomplete")),
    }
}
