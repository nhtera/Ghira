// SPDX-License-Identifier: Apache-2.0
//! Paired devices (slice 15-C1): pins, the pair PSK, wipe-pending state and
//! the per-peer cursors (`devices` table, doc 07 §3.1).
//!
//! The pair PSK is sealed under a key derived from the KeyRing's master key
//! (AAD `device:<gid>`) and never leaves this module in the clear except as
//! [`zeroize::Zeroizing`]. It is derived from the master key, not the wrap
//! secret, because the wrap secret rotates on every meeting delete and only
//! meeting and voice keys are re-wrapped by that rotation.

use hkdf::Hkdf;
use rusqlite::{OptionalExtension, Row, params};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::keys::KeyRing;
use crate::rowcrypt::{self, Dek};
use crate::store::{Store, check_gid, now_ms};
use crate::{Result, StoreError};

const PSK_KEY_INFO: &[u8] = b"ghira/device-psk/v1";
const DEVICE_COLS: &str = "id, gid, name, platform, role, static_pub, state, paired_at,
                           last_seen, last_addr, push_seq, pull_feed_id, pull_seq";

fn psk_key(ring: &KeyRing) -> Dek {
    let mut out = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(None, ring.master().as_bytes())
        .expand(PSK_KEY_INFO, out.as_mut_slice())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    Dek::from_bytes(*out)
}

fn psk_aad(gid: &str) -> Vec<u8> {
    format!("device:{gid}").into_bytes()
}

fn not_found(gid: &str) -> StoreError {
    StoreError::NotFound {
        kind: "device",
        gid: gid.to_string(),
    }
}

fn device_from_row(r: &Row) -> rusqlite::Result<Device> {
    let bad = |what: &str| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            what.to_string().into(),
        )
    };
    let role: String = r.get(4)?;
    let state: String = r.get(6)?;
    let key: Vec<u8> = r.get(5)?;
    Ok(Device {
        id: r.get(0)?,
        gid: r.get(1)?,
        name: r.get(2)?,
        platform: r.get(3)?,
        role: match role.as_str() {
            "hub" => DeviceRole::Hub,
            "spoke" => DeviceRole::Spoke,
            _ => return Err(bad("device role")),
        },
        static_pub: key.try_into().map_err(|_| bad("device key length"))?,
        state: match state.as_str() {
            "paired" => DeviceState::Paired,
            "wipe_pending" => DeviceState::WipePending,
            _ => return Err(bad("device state")),
        },
        paired_at: r.get(7)?,
        last_seen: r.get(8)?,
        last_addr: r.get(9)?,
        push_seq: r.get(10)?,
        pull_feed_id: r.get(11)?,
        pull_seq: r.get(12)?,
    })
}

/// Settings key of a peer's remembered mass-delete confirmation (D13).
fn mass_delete_key(gid: &str) -> String {
    format!("sync.mass_delete_ok:{gid}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceRole {
    Hub,
    Spoke,
}

impl DeviceRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hub => "hub",
            Self::Spoke => "spoke",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    Paired,
    /// Only a session that delivers `Wipe` is allowed.
    WipePending,
}

/// A paired peer.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: i64,
    pub gid: String,
    pub name: String,
    pub platform: String,
    /// The peer's role.
    pub role: DeviceRole,
    pub static_pub: [u8; 32],
    pub state: DeviceState,
    pub paired_at: i64,
    pub last_seen: Option<i64>,
    pub last_addr: Option<String>,
    /// Highest local `sync_log.seq` the peer acked.
    pub push_seq: i64,
    /// The peer's feed the pull cursor belongs to, and the cursor.
    pub pull_feed_id: Option<String>,
    pub pull_seq: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewDevice {
    pub gid: String,
    pub name: String,
    pub platform: String,
    pub role: DeviceRole,
    pub static_pub: [u8; 32],
}

impl Store {
    /// Pins a peer (committing a pairing). A second pin of the same gid or key
    /// is [`StoreError::Duplicate`].
    pub fn pin_device(&self, device: &NewDevice, pair_psk: &[u8; 32]) -> Result<Device> {
        check_gid(&device.gid)?;
        if device.name.trim().is_empty() || device.name.len() > 256 || device.platform.len() > 64 {
            return Err(StoreError::Invalid("device name or platform".into()));
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let taken: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM devices WHERE gid = ?1 OR static_pub = ?2)
                 OR EXISTS (SELECT 1 FROM settings
                            WHERE key = 'sync.device_gid' AND value_json = json_quote(?1))",
            params![device.gid, device.static_pub.as_slice()],
            |r| r.get(0),
        )?;
        if taken {
            return Err(StoreError::Duplicate { kind: "device" });
        }
        let wrapped = {
            let ring = self.ring();
            rowcrypt::seal(&psk_key(&ring), pair_psk, &psk_aad(&device.gid))
        };
        tx.execute(
            "INSERT INTO devices (gid, name, platform, role, static_pub, pair_psk_wrapped, paired_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                device.gid,
                device.name,
                device.platform,
                device.role.as_str(),
                device.static_pub.as_slice(),
                wrapped,
                now_ms()
            ],
        )?;
        let d = tx.query_row(
            &format!("SELECT {DEVICE_COLS} FROM devices WHERE gid = ?1"),
            [&device.gid],
            device_from_row,
        )?;
        tx.commit()?;
        Ok(d)
    }

    /// Removes the pin and the PSK (and what was exchanged with the peer is
    /// forgotten as a wipe scope). Data stays. Unpinning an unknown gid is a
    /// no-op.
    pub fn unpin_device(&self, gid: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM devices WHERE gid = ?1", [gid])?;
        tx.execute(
            "DELETE FROM settings WHERE key = ?1",
            [mass_delete_key(gid)],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Moves a pin to `wipe_pending` ("Unpair and wipe").
    pub fn set_wipe_pending(&self, gid: &str) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE devices SET state = 'wipe_pending' WHERE gid = ?1",
            [gid],
        )?;
        if n == 0 {
            return Err(not_found(gid));
        }
        Ok(())
    }

    /// Records a session: `last_seen = now`, and the address that worked.
    pub fn touch_device(&self, gid: &str, addr: Option<&str>) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE devices SET last_seen = ?2, last_addr = COALESCE(?3, last_addr)
             WHERE gid = ?1",
            params![gid, now_ms(), addr],
        )?;
        if n == 0 {
            return Err(not_found(gid));
        }
        Ok(())
    }

    pub fn devices(&self) -> Result<Vec<Device>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("SELECT {DEVICE_COLS} FROM devices ORDER BY id"))?;
        let rows = stmt.query_map([], device_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn device(&self, gid: &str) -> Result<Option<Device>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {DEVICE_COLS} FROM devices WHERE gid = ?1"),
                [gid],
                device_from_row,
            )
            .optional()?)
    }

    /// The pinned device with this static key, if any (a responder's first
    /// lookup after Noise message 1).
    pub fn device_by_key(&self, static_pub: &[u8; 32]) -> Result<Option<Device>> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT {DEVICE_COLS} FROM devices WHERE static_pub = ?1"),
                [static_pub.as_slice()],
                device_from_row,
            )
            .optional()?)
    }

    /// The unwrapped pair PSK of a pinned device.
    pub fn pair_psk(&self, gid: &str) -> Result<Zeroizing<[u8; 32]>> {
        let wrapped: Vec<u8> = self
            .conn()
            .query_row(
                "SELECT pair_psk_wrapped FROM devices WHERE gid = ?1",
                [gid],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| not_found(gid))?;
        let plain = {
            let ring = self.ring();
            Zeroizing::new(rowcrypt::open(&psk_key(&ring), &wrapped, &psk_aad(gid))?)
        };
        let arr: [u8; 32] = plain
            .as_slice()
            .try_into()
            .map_err(|_| StoreError::Decrypt)?;
        Ok(Zeroizing::new(arr))
    }

    /// Stores the cursors after a peer acked: push (`push_seq`) and pull
    /// (`pull_feed_id`, `pull_seq`). A pull feed id that differs from the one
    /// stored means the peer's log was replaced: the caller passes `pull_seq`
    /// 0 then (see [`Device::pull_feed_id`]).
    pub fn set_cursors(
        &self,
        gid: &str,
        push_seq: i64,
        pull_feed_id: Option<&str>,
        pull_seq: i64,
    ) -> Result<()> {
        if push_seq < 0 || pull_seq < 0 {
            return Err(StoreError::Invalid("negative cursor".into()));
        }
        let n = self.conn().execute(
            "UPDATE devices SET push_seq = ?2, pull_feed_id = ?3, pull_seq = ?4 WHERE gid = ?1",
            params![gid, push_seq, pull_feed_id, pull_seq],
        )?;
        if n == 0 {
            return Err(not_found(gid));
        }
        Ok(())
    }

    /// The pull cursor to ask `gid` for, given the feed id it announced:
    /// 0 when it is not the feed the stored cursor belongs to (a full resync).
    pub fn pull_cursor_for(&self, gid: &str, peer_feed_id: &str) -> Result<i64> {
        let d = self.device(gid)?.ok_or_else(|| not_found(gid))?;
        Ok(if d.pull_feed_id.as_deref() == Some(peer_feed_id) {
            d.pull_seq
        } else {
            0
        })
    }

    /// Remembers that the user confirmed a mass delete from this peer, so
    /// the resent batch applies without asking again (amendment 5, D13).
    pub fn set_mass_delete_confirmed(&self, gid: &str, confirmed: bool) -> Result<()> {
        if self.device(gid)?.is_none() {
            return Err(not_found(gid));
        }
        if confirmed {
            self.set_setting(&mass_delete_key(gid), &serde_json::Value::Bool(true))
        } else {
            self.conn().execute(
                "DELETE FROM settings WHERE key = ?1",
                [mass_delete_key(gid)],
            )?;
            Ok(())
        }
    }

    pub fn mass_delete_confirmed(&self, gid: &str) -> Result<bool> {
        Ok(self.get_setting(&mass_delete_key(gid))? == Some(serde_json::Value::Bool(true)))
    }
}
