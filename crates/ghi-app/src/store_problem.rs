// SPDX-License-Identifier: Apache-2.0
//! Why the encrypted store can't be opened, as a stable code the UI turns
//! into words (`mobile.storeProblem.*`). Never an English string.
//!
//! Only [`StoreProblem::KeyMissing`] and [`StoreProblem::Damaged`] mean the
//! data is gone for good ([`StoreProblem::allows_start_fresh`]); every other
//! code can pass (an unlock, free space, a retry), so nothing offers to erase.

use ghi_store::StoreError;
use rusqlite::ErrorCode;
use serde::Serialize;
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum StoreProblem {
    /// The data is there, its key is not (the key stays on its device).
    KeyMissing,
    /// The key needs the user's presence (the phone or the app is locked, or
    /// the prompt was cancelled or denied).
    KeyLocked,
    /// The OS key store failed.
    Keystore,
    /// The database is not readable with its key (`SQLITE_NOTADB`, a failed
    /// authentication) or is corrupt (`SQLITE_CORRUPT`; it doesn't heal).
    Damaged,
    /// An upgrade of the database failed (and was rolled back).
    Migration,
    /// The files couldn't be read or written (I/O, no space, can't open).
    Disk,
    /// Anything else about the open itself (busy, a malformed request): a retry may pass.
    Other,
    /// The store opened but starting on top of it failed (recovery, job runner).
    Startup,
}

impl StoreProblem {
    pub fn of(e: &StoreError) -> Self {
        match e {
            StoreError::KeyMissing => Self::KeyMissing,
            StoreError::KeyLocked => Self::KeyLocked,
            StoreError::Keystore { .. } | StoreError::Unsupported(_) => Self::Keystore,
            StoreError::Decrypt => Self::Damaged,
            StoreError::Db(rusqlite::Error::SqliteFailure(f, _)) => match f.code {
                ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt => Self::Damaged,
                ErrorCode::DiskFull
                | ErrorCode::CannotOpen
                | ErrorCode::SystemIoFailure
                | ErrorCode::ReadOnly => Self::Disk,
                _ => Self::Other,
            },
            StoreError::Migration { .. } => Self::Migration,
            StoreError::Io(_) => Self::Disk,
            _ => Self::Other,
        }
    }

    /// The data can't be read and no retry will change that: the only way on
    /// is to start over.
    pub fn allows_start_fresh(self) -> bool {
        matches!(self, Self::KeyMissing | Self::Damaged)
    }

    /// The stable code (the same text the UI receives).
    pub fn code(self) -> &'static str {
        match self {
            Self::KeyMissing => "keyMissing",
            Self::KeyLocked => "keyLocked",
            Self::Keystore => "keystore",
            Self::Damaged => "damaged",
            Self::Migration => "migration",
            Self::Disk => "disk",
            Self::Other => "other",
            Self::Startup => "startup",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(code: ErrorCode, extended: i32) -> StoreError {
        StoreError::Db(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code,
                extended_code: extended,
            },
            None,
        ))
    }

    #[test]
    fn each_store_error_maps_to_a_stable_code() {
        let cases = [
            (StoreError::KeyMissing, StoreProblem::KeyMissing),
            (StoreError::KeyLocked, StoreProblem::KeyLocked),
            (
                StoreError::Keystore {
                    detail: "-25300".into(),
                },
                StoreProblem::Keystore,
            ),
            (StoreError::Unsupported("x"), StoreProblem::Keystore),
            (StoreError::Decrypt, StoreProblem::Damaged),
            (db(ErrorCode::NotADatabase, 26), StoreProblem::Damaged),
            (db(ErrorCode::DatabaseCorrupt, 11), StoreProblem::Damaged),
            (db(ErrorCode::DiskFull, 13), StoreProblem::Disk),
            (db(ErrorCode::CannotOpen, 14), StoreProblem::Disk),
            (db(ErrorCode::SystemIoFailure, 10), StoreProblem::Disk),
            // Retryable or not understood: never "damaged".
            (db(ErrorCode::DatabaseBusy, 5), StoreProblem::Other),
            (db(ErrorCode::DatabaseLocked, 6), StoreProblem::Other),
            (
                StoreError::Migration {
                    version: 7,
                    detail: "x".into(),
                },
                StoreProblem::Migration,
            ),
            (
                StoreError::Io(std::io::Error::other("x")),
                StoreProblem::Disk,
            ),
            (StoreError::Invalid("x".into()), StoreProblem::Other),
        ];
        for (e, want) in cases {
            assert_eq!(StoreProblem::of(&e), want, "{e}");
        }
    }

    #[test]
    fn only_an_unreadable_store_may_be_started_fresh() {
        use StoreProblem::*;
        for p in [KeyMissing, Damaged] {
            assert!(p.allows_start_fresh(), "{p:?}");
        }
        for p in [KeyLocked, Keystore, Migration, Disk, Other, Startup] {
            assert!(!p.allows_start_fresh(), "{p:?}");
        }
    }

    #[test]
    fn codes_serialize_in_camel_case_and_match_code() {
        use StoreProblem::*;
        for p in [
            KeyMissing, KeyLocked, Keystore, Damaged, Migration, Disk, Other, Startup,
        ] {
            assert_eq!(
                serde_json::to_string(&p).unwrap(),
                format!("\"{}\"", p.code())
            );
        }
    }
}
