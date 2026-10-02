// SPDX-License-Identifier: Apache-2.0
//! SQLCipher storage, Vietnamese-folded FTS5 search, audio bundles and key management.
//!
//! `ghi-store` is the single owner of persisted data:
//! - [`db`] / [`migrate`]: the encrypted SQLite database (SQLCipher, key
//!   derived from the master key) and its migrations with snapshots.
//! - [`rowcrypt`]: per-meeting data keys (DEKs) and text-column encryption.
//! - [`bundle`]: the only audio writer: encrypted ~1 s pages, crash-safe,
//!   random access for playback.
//! - [`embeddings`]: sealed transcript-chunk vectors for semantic search.
//! - [`fold`] / [`search`]: Vietnamese accent-insensitive FTS5 search over a
//!   contentless index, with highlights on the original text.
//! - [`keys`]: the master key in the OS keystore; [`recovery`]: the optional
//!   24-word recovery key; [`export`]: password-encrypted archives.
//! - [`store`]: the facade the app uses, including crypto-shred delete,
//!   tombstones, jobs and retention.
//!
//! Library code never prints (release builds abort on panic).

pub mod anchors;
pub mod backup;
pub mod bundle;
pub mod db;
pub mod edits;
pub mod embeddings;
pub mod export;
pub mod fold;
pub mod jobs;
pub mod keys;
pub mod migrate;
pub mod recovery;
pub mod retention;
pub mod rowcrypt;
pub mod search;
pub mod store;
pub mod tombstones;

use std::fmt;

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Errors from the store. Messages never contain user content.
#[derive(Debug)]
pub enum StoreError {
    /// SQLite / SQLCipher failure (including a wrong database key).
    Db(rusqlite::Error),
    Io(std::io::Error),
    /// Authentication failed: wrong key, tampered or truncated data.
    Decrypt,
    /// The OS keystore failed; `detail` is the platform's status text.
    Keystore {
        detail: String,
    },
    /// The keystore needs user presence and it was cancelled or denied.
    KeyLocked,
    /// This platform has no keystore implementation yet.
    Unsupported(&'static str),
    NotFound {
        kind: &'static str,
        gid: String,
    },
    /// A migration failed; the pre-migration snapshot was restored.
    Migration {
        version: u32,
        detail: String,
    },
    /// Malformed input (bad recovery phrase, bad archive, ...).
    Invalid(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Db(e) => write!(f, "database: {e}"),
            StoreError::Io(e) => write!(f, "file: {e}"),
            StoreError::Decrypt => f.write_str("decryption failed (wrong key or damaged data)"),
            StoreError::Keystore { detail } => write!(f, "keystore: {detail}"),
            StoreError::KeyLocked => {
                f.write_str("the key needs you to unlock (cancelled or denied)")
            }
            StoreError::Unsupported(what) => write!(f, "not supported on this platform: {what}"),
            StoreError::NotFound { kind, gid } => write!(f, "{kind} {gid} not found"),
            StoreError::Migration { version, detail } => {
                write!(
                    f,
                    "migration {version} failed and was rolled back: {detail}"
                )
            }
            StoreError::Invalid(what) => write!(f, "invalid input: {what}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Db(e)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// A new globally unique id for a syncable row (UUIDv7: time-ordered).
pub fn new_gid() -> String {
    uuid::Uuid::now_v7().to_string()
}
