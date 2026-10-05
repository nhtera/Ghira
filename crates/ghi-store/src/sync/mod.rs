// SPDX-License-Identifier: Apache-2.0
//! The store side of LAN sync (phase 15, doc 07 §7).
//!
//! Layout (each file is owned by one slice; phase 15 plan §4):
//! - C1: [`feed`] (`sync_log` reads, cursors, `feed_id`), [`devices`] (pins),
//!   [`keys`] (DEK transfer), [`leases`], [`wipe`], [`settings`].
//! - C2: [`records`] (typed wire records), [`apply`] (tombstones, rows),
//!   [`rules`] (version and field rules), [`conflicts`], [`pending`].
//!
//! In this slice (W0) the bodies return [`StoreError::NotYet`]; the
//! signatures are the contract the other slices build on. Only
//! [`Store::observe_lamport`] is real.
//!
//! Peers are named by their device gid on this API; `origin` columns hold
//! `devices.id` (NULL = this device), and the conversion stays inside this
//! module.

pub mod apply;
pub mod audio;
pub mod conflicts;
pub mod devices;
pub mod feed;
pub mod keys;
pub mod leases;
pub mod pending;
pub mod records;
pub mod rules;
pub mod settings;
pub mod wipe;

use rusqlite::Connection;

use crate::Result;
use crate::store::Store;

impl Store {
    /// Sets the Lamport clock to `max(clock, remote)`: called for every remote
    /// row applied, so a later local write sorts after it.
    pub fn observe_lamport(&self, remote: i64) -> Result<()> {
        observe_lamport(&self.conn(), remote)
    }
}

/// [`Store::observe_lamport`] on an open connection or transaction.
pub(crate) fn observe_lamport(conn: &Connection, remote: i64) -> Result<()> {
    conn.execute(
        "UPDATE settings SET value_json = CAST(?1 AS TEXT)
         WHERE key = 'lamport' AND CAST(value_json AS INTEGER) < ?1",
        [remote],
    )?;
    Ok(())
}
