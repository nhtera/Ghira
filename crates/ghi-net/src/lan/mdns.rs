// SPDX-License-Identifier: Apache-2.0
//! mDNS for the desktop and the CLI (cargo feature `mdns`; the iOS build
//! doesn't enable it and uses `NWBrowser` through [`super::PushedDiscovery`]).
//! Advertising: `_ghi._tcp.local.` with a random `ghi-<8hex>` instance and
//! host name per launch, TXT `v=1` and `a=<ip:port,...>` (LAN addresses only),
//! restricted to LAN interfaces; `unregister` before `shutdown`. Browsing keeps
//! at most [`super::MAX_CANDIDATES`] LAN candidates. Bodies land in 15-E.

use std::net::{IpAddr, SocketAddr};

use super::{Discovery, LanError, is_lan};

/// Advertises the hub while sync is on and a listener is open.
#[derive(Debug)]
pub struct MdnsAdvertiser {
    _private: (),
}

impl MdnsAdvertiser {
    /// Starts advertising `port` on the given LAN addresses.
    pub fn start(addrs: &[IpAddr], _port: u16) -> Result<MdnsAdvertiser, LanError> {
        if let Some(bad) = addrs.iter().find(|ip| !is_lan(**ip)) {
            return Err(LanError::NotLan(*bad));
        }
        Err(LanError::NotYet("lan::MdnsAdvertiser::start"))
    }
}

/// Browses for a hub (the CLI; iOS uses `NWBrowser`).
#[derive(Debug)]
pub struct MdnsDiscovery {
    _private: (),
}

impl MdnsDiscovery {
    pub fn start() -> Result<MdnsDiscovery, LanError> {
        Err(LanError::NotYet("lan::MdnsDiscovery::start"))
    }
}

impl Discovery for MdnsDiscovery {
    fn candidates(&self) -> Vec<SocketAddr> {
        Vec::new()
    }
}
