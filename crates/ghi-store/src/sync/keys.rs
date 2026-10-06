// SPDX-License-Identifier: Apache-2.0
//! Meeting key transfer (slice 15-C1, doc 07 §7.6).
//!
//! A DEK goes to a peer once, inside the meeting record in the Noise channel
//! (`peer_meetings.key_sent`). The receiver refuses it for a tombstoned gid,
//! requires it to equal any DEK it already holds (T10), and stores it only
//! wrapped under its own KeyRing. Voice profile keys do not sync (v1).

use rusqlite::{Connection, OptionalExtension, params};
use zeroize::Zeroizing;

use crate::store::{Store, check_gid};
use crate::{Result, StoreError, tombstones};

fn device_id(conn: &Connection, gid: &str) -> Result<i64> {
    conn.query_row("SELECT id FROM devices WHERE gid = ?1", [gid], |r| r.get(0))
        .optional()?
        .ok_or_else(|| StoreError::NotFound {
            kind: "device",
            gid: gid.to_string(),
        })
}

/// Records `meeting_gid` as exchanged with the device (key not sent yet).
fn note_exchange(conn: &Connection, device: i64, meeting_gid: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO peer_meetings (device_id, meeting_gid, key_sent) VALUES (?1, ?2, 0)",
        params![device, meeting_gid],
    )?;
    Ok(())
}

impl Store {
    /// The DEK to put in `meeting_gid`'s record for `device_gid`, or `None` if
    /// it was already sent. Records the meeting in `peer_meetings`. The key
    /// counts as sent only after [`Store::mark_key_sent`] (the peer's ack).
    pub fn meeting_dek_for_peer(
        &self,
        device_gid: &str,
        meeting_gid: &str,
    ) -> Result<Option<Zeroizing<[u8; 32]>>> {
        check_gid(meeting_gid)?;
        let conn = self.conn();
        let device = device_id(&conn, device_gid)?;
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        note_exchange(&conn, device, meeting_gid)?;
        let sent: bool = conn.query_row(
            "SELECT key_sent FROM peer_meetings WHERE device_id = ?1 AND meeting_gid = ?2",
            params![device, meeting_gid],
            |r| r.get(0),
        )?;
        if sent {
            return Ok(None);
        }
        let dek = self.dek(&conn, m.id)?;
        Ok(Some(Zeroizing::new(*dek.as_bytes())))
    }

    /// Marks the DEK of a meeting as delivered (after the peer's ack).
    pub fn mark_key_sent(&self, device_gid: &str, meeting_gid: &str) -> Result<()> {
        let conn = self.conn();
        let device = device_id(&conn, device_gid)?;
        note_exchange(&conn, device, meeting_gid)?;
        conn.execute(
            "UPDATE peer_meetings SET key_sent = 1 WHERE device_id = ?1 AND meeting_gid = ?2",
            params![device, meeting_gid],
        )?;
        Ok(())
    }

    /// Takes the DEK a peer sent for `meeting_gid`, whose row exists here:
    /// refused for a tombstoned gid, and it must equal the DEK already held
    /// (T10). Records the meeting as exchanged with `from_device`, which has
    /// the key by definition. A meeting row that does not exist yet takes its
    /// key from [`Store::incoming_dek_wrapped`] inside the inserting
    /// transaction instead.
    pub fn accept_dek(&self, meeting_gid: &str, dek: &[u8; 32], from_device: &str) -> Result<()> {
        let dek = Zeroizing::new(*dek);
        check_gid(meeting_gid)?;
        let conn = self.conn();
        let device = device_id(&conn, from_device)?;
        if self
            .incoming_dek_wrapped(&conn, meeting_gid, &dek)?
            .is_some()
        {
            // No row holds a key yet: the key belongs in the insert.
            return Err(StoreError::NotFound {
                kind: "meeting",
                gid: meeting_gid.to_string(),
            });
        }
        note_exchange(&conn, device, meeting_gid)?;
        conn.execute(
            "UPDATE peer_meetings SET key_sent = 1 WHERE device_id = ?1 AND meeting_gid = ?2",
            params![device, meeting_gid],
        )?;
        Ok(())
    }

    /// The checks of a received DEK, on the caller's connection or transaction
    /// (the merge engine calls this when it inserts a meeting row):
    ///
    /// - tombstoned gid: [`StoreError::Tombstoned`];
    /// - a live key already held: it must be equal, else [`StoreError::Invalid`]
    ///   (T10), and the result is `None`;
    /// - otherwise the key wrapped under this device's ring, for
    ///   `meetings.dek_wrapped`.
    ///
    /// A shredded row (zeroed key) is refused like a tombstoned gid: its
    /// delete is in progress.
    pub(crate) fn incoming_dek_wrapped(
        &self,
        conn: &Connection,
        meeting_gid: &str,
        dek: &[u8; 32],
    ) -> Result<Option<Vec<u8>>> {
        tombstones::assert_live(conn, meeting_gid)?;
        let held: Option<Vec<u8>> = conn
            .query_row(
                "SELECT dek_wrapped FROM meetings WHERE gid = ?1",
                [meeting_gid],
                |r| r.get(0),
            )
            .optional()?;
        let ring = self.ring();
        match held {
            Some(w) if w.iter().all(|b| *b == 0) => Err(StoreError::Tombstoned {
                gid: meeting_gid.to_string(),
            }),
            Some(w) => {
                let existing = ring.unwrap_dek(&w, meeting_gid)?;
                // Constant-time compare: both are secrets.
                let diff = existing
                    .as_bytes()
                    .iter()
                    .zip(dek.iter())
                    .fold(0u8, |a, (x, y)| a | (x ^ y));
                if diff != 0 {
                    return Err(StoreError::Invalid(
                        "meeting key differs from the one held".into(),
                    ));
                }
                Ok(None)
            }
            None => Ok(Some(
                ring.wrap_dek(&crate::rowcrypt::Dek::from_bytes(*dek), meeting_gid),
            )),
        }
    }

    /// The gid of the paired device that recorded the meeting; `None` when it
    /// was recorded or imported here (or is unknown).
    pub fn meeting_audio_origin_gid(&self, meeting_gid: &str) -> Result<Option<String>> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT d.gid FROM meetings m JOIN devices d ON d.id = m.audio_origin
                 WHERE m.gid = ?1",
                [meeting_gid],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Meetings exchanged with a peer, in either direction (a Wipe's scope).
    pub fn peer_meetings(&self, device_gid: &str) -> Result<Vec<String>> {
        let conn = self.conn();
        let device = device_id(&conn, device_gid)?;
        let mut stmt = conn.prepare(
            "SELECT meeting_gid FROM peer_meetings WHERE device_id = ?1 ORDER BY meeting_gid",
        )?;
        let rows = stmt.query_map([device], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
