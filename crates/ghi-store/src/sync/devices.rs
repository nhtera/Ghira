// SPDX-License-Identifier: Apache-2.0
//! Paired devices (slice 15-C1): pins, the pair PSK, wipe-pending state and
//! the per-peer cursors (`devices` table, doc 07 §3.1).
//!
//! The pair PSK is wrapped by the KeyRing like a DEK (AAD `device:<gid>`) and
//! never leaves this module in the clear except as [`zeroize::Zeroizing`].

use zeroize::Zeroizing;

use super::not_yet;
use crate::Result;
use crate::store::Store;

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
    /// is an error.
    pub fn pin_device(&self, _device: &NewDevice, _pair_psk: &[u8; 32]) -> Result<Device> {
        not_yet("sync::devices::pin_device")
    }

    /// Removes the pin and the PSK. Data stays.
    pub fn unpin_device(&self, _gid: &str) -> Result<()> {
        not_yet("sync::devices::unpin_device")
    }

    /// Moves a pin to `wipe_pending` ("Unpair and wipe").
    pub fn set_wipe_pending(&self, _gid: &str) -> Result<()> {
        not_yet("sync::devices::set_wipe_pending")
    }

    /// Records a session: `last_seen = now`, and the address that worked.
    pub fn touch_device(&self, _gid: &str, _addr: Option<&str>) -> Result<()> {
        not_yet("sync::devices::touch_device")
    }

    pub fn devices(&self) -> Result<Vec<Device>> {
        not_yet("sync::devices::devices")
    }

    pub fn device(&self, _gid: &str) -> Result<Option<Device>> {
        not_yet("sync::devices::device")
    }

    /// The pinned device with this static key, if any (a responder's first
    /// lookup after Noise message 1).
    pub fn device_by_key(&self, _static_pub: &[u8; 32]) -> Result<Option<Device>> {
        not_yet("sync::devices::device_by_key")
    }

    /// The unwrapped pair PSK of a pinned device.
    pub fn pair_psk(&self, _gid: &str) -> Result<Zeroizing<[u8; 32]>> {
        not_yet("sync::devices::pair_psk")
    }

    /// Stores the cursors after a peer acked: push (`push_seq`) and pull
    /// (`pull_feed_id`, `pull_seq`).
    pub fn set_cursors(
        &self,
        _gid: &str,
        _push_seq: i64,
        _pull_feed_id: Option<&str>,
        _pull_seq: i64,
    ) -> Result<()> {
        not_yet("sync::devices::set_cursors")
    }
}
