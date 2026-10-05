// SPDX-License-Identifier: Apache-2.0
//! mDNS for the desktop and the CLI (cargo feature `mdns`; the iOS build
//! doesn't enable it and uses `NWBrowser` through [`super::PushedDiscovery`]).
//! Advertising: `_ghi._tcp.local.` with a random `ghi-<8hex>` instance and
//! host name per launch, TXT `v=1` and `a=<ip:port,...>` (LAN addresses only),
//! restricted to LAN interfaces (spike Q10: `disable_interface(All)` then
//! `enable_interface(Addr)` per address); `unregister` before `shutdown`, or
//! stale records stay cached. Browsing keeps at most [`super::MAX_CANDIDATES`]
//! LAN candidates and ignores every other address (link-local included).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent, ServiceInfo};

use super::{Discovery, LanError, is_lan, keep_lan, lan_addrs};

const SERVICE_TYPE: &str = "_ghi._tcp.local.";
/// One TXT string holds at most 255 bytes.
const TXT_MAX: usize = 255;
const GOODBYE_WAIT: Duration = Duration::from_secs(1);

fn mdns_err(e: impl std::fmt::Display) -> LanError {
    LanError::Mdns(e.to_string())
}

/// Only the LAN addresses' interfaces take part: no VPN, `utun`, CGNAT or
/// loopback.
fn restrict_to(daemon: &ServiceDaemon, addrs: &[IpAddr]) -> Result<(), LanError> {
    daemon.disable_interface(IfKind::All).map_err(mdns_err)?;
    for ip in addrs {
        daemon
            .enable_interface(IfKind::Addr(*ip))
            .map_err(mdns_err)?;
    }
    Ok(())
}

fn random_name() -> Result<String, LanError> {
    let mut b = [0u8; 4];
    getrandom::getrandom(&mut b).map_err(mdns_err)?;
    Ok(format!(
        "ghi-{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3]
    ))
}

/// Advertises the hub while sync is on and a listener is open.
pub struct MdnsAdvertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

impl std::fmt::Debug for MdnsAdvertiser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MdnsAdvertiser")
            .field("fullname", &self.fullname)
            .finish()
    }
}

impl MdnsAdvertiser {
    /// Starts advertising `port` on the given LAN addresses.
    pub fn start(addrs: &[IpAddr], port: u16) -> Result<MdnsAdvertiser, LanError> {
        if let Some(bad) = addrs.iter().find(|ip| !is_lan(**ip)) {
            return Err(LanError::NotLan(*bad));
        }
        if addrs.is_empty() {
            return Err(LanError::NoAddress);
        }
        let name = random_name()?;
        let host = format!("{name}.local.");
        let mut a = String::new();
        for ip in addrs {
            let one = SocketAddr::new(*ip, port).to_string();
            if a.len() + one.len() + 3 > TXT_MAX {
                break;
            }
            if !a.is_empty() {
                a.push(',');
            }
            a.push_str(&one);
        }
        let props: HashMap<String, String> =
            HashMap::from([("v".into(), "1".into()), ("a".into(), a)]);
        let info =
            ServiceInfo::new(SERVICE_TYPE, &name, &host, addrs, port, props).map_err(mdns_err)?;
        let fullname = info.get_fullname().to_owned();
        let daemon = ServiceDaemon::new().map_err(mdns_err)?;
        let started =
            restrict_to(&daemon, addrs).and_then(|()| daemon.register(info).map_err(mdns_err));
        if let Err(e) = started {
            let _ = daemon.shutdown();
            return Err(e);
        }
        Ok(MdnsAdvertiser { daemon, fullname })
    }

    /// The advertised service name (`ghi-<8hex>._ghi._tcp.local.`).
    pub fn fullname(&self) -> &str {
        &self.fullname
    }
}

impl Drop for MdnsAdvertiser {
    fn drop(&mut self) {
        // Goodbye first, or neighbours keep the record until its TTL ends.
        if let Ok(done) = self.daemon.unregister(&self.fullname) {
            let _ = done.recv_timeout(GOODBYE_WAIT);
        }
        if let Ok(done) = self.daemon.shutdown() {
            let _ = done.recv_timeout(GOODBYE_WAIT);
        }
    }
}

/// Browses for a hub (the CLI; iOS uses `NWBrowser`).
pub struct MdnsDiscovery {
    daemon: ServiceDaemon,
    found: Arc<Mutex<HashMap<String, Vec<SocketAddr>>>>,
}

impl std::fmt::Debug for MdnsDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MdnsDiscovery")
    }
}

impl MdnsDiscovery {
    pub fn start() -> Result<MdnsDiscovery, LanError> {
        let addrs = lan_addrs();
        if addrs.is_empty() {
            return Err(LanError::NoAddress);
        }
        let daemon = ServiceDaemon::new().map_err(mdns_err)?;
        let events = match restrict_to(&daemon, &addrs)
            .and_then(|()| daemon.browse(SERVICE_TYPE).map_err(mdns_err))
        {
            Ok(rx) => rx,
            Err(e) => {
                let _ = daemon.shutdown();
                return Err(e);
            }
        };
        let found: Arc<Mutex<HashMap<String, Vec<SocketAddr>>>> = Arc::default();
        let sink = Arc::clone(&found);
        std::thread::spawn(move || {
            // Ends when the daemon shuts down and drops the sender.
            while let Ok(event) = events.recv() {
                let mut map = sink.lock().unwrap_or_else(|p| p.into_inner());
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let port = info.get_port();
                        let lan: Vec<SocketAddr> = info
                            .get_addresses()
                            .iter()
                            .map(|a| a.to_ip_addr())
                            .filter(|ip| is_lan(*ip))
                            .map(|ip| SocketAddr::new(ip, port))
                            .collect();
                        if lan.is_empty() {
                            map.remove(info.get_fullname());
                        } else {
                            map.insert(info.get_fullname().to_owned(), lan);
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        map.remove(&fullname);
                    }
                    _ => {}
                }
            }
        });
        Ok(MdnsDiscovery { daemon, found })
    }
}

impl Discovery for MdnsDiscovery {
    fn candidates(&self) -> Vec<SocketAddr> {
        let map = self.found.lock().unwrap_or_else(|p| p.into_inner());
        let mut names: Vec<&String> = map.keys().collect();
        names.sort();
        keep_lan(names.into_iter().flat_map(|n| map[n].iter().copied()))
    }
}

impl Drop for MdnsDiscovery {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        if let Ok(done) = self.daemon.shutdown() {
            let _ = done.recv_timeout(GOODBYE_WAIT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::test_lan_ip;
    use super::*;

    #[test]
    fn refuses_non_lan_addresses_and_empty_lists() {
        for ip in [
            "127.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "8.8.8.8",
            "fe80::1",
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(matches!(
                MdnsAdvertiser::start(&[ip], 1),
                Err(LanError::NotLan(_))
            ));
        }
        assert!(matches!(
            MdnsAdvertiser::start(&[], 1),
            Err(LanError::NoAddress)
        ));
    }

    #[test]
    fn advertise_then_browse_finds_only_lan_addresses() {
        let Some(ip) = test_lan_ip() else { return };
        let ad = MdnsAdvertiser::start(&[ip], 48_731).unwrap();
        assert!(ad.fullname().starts_with("ghi-") && ad.fullname().ends_with(SERVICE_TYPE));
        let browser = MdnsDiscovery::start().unwrap();
        let want = SocketAddr::new(ip, 48_731);
        let mut found = Vec::new();
        for _ in 0..100 {
            found = browser.candidates();
            if found.contains(&want) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(found.contains(&want), "not discovered: {found:?}");
        assert!(found.iter().all(|a| is_lan(a.ip())));
        assert!(found.len() <= super::super::MAX_CANDIDATES);
    }
}
