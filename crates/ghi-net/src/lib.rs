// SPDX-License-Identifier: Apache-2.0
//! The only network egress point of Ghira (red-team finding RT-6).
//!
//! Every network path (cloud LLM, model download, update check, calendar OAuth,
//! LAN sync listener) must go through this crate. CI rejects HTTP clients and
//! sockets anywhere else (`deny.toml` bans + `tools/scripts/check-net-egress.sh`).
//! The policy model lives here; [`cloud`] is the HTTPS client for cloud AI,
//! gated by a per-request [`CloudGrant`] (phase 6). Per-class switches for model
//! downloads and the update check, and the LAN listener guard (phase 15), build
//! on it.

mod cloud;
pub mod fetch;
mod ip;
mod secret;

pub use cloud::{
    CloudGrant, GRANT_TTL, HttpResponse, MAX_ATTEMPTS, MeetingGate, NetError, connections_opened,
    send, sha256_hex,
};
pub use ip::{is_lan, is_public};
pub use secret::{Headers, Secret};

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// User-selected network mode, exactly as defined in `PRIVACY.md` and doc 05 §3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetPolicy {
    /// Content-free traffic to allowlisted hosts (model downloads, update
    /// check) plus sync with paired devices on private networks.
    #[default]
    Default,
    /// "Strict offline": all internet traffic blocked; sync with paired devices
    /// on private address ranges stays allowed.
    StrictOffline,
}

/// Where a connection would go. Always derived here from the host, never
/// supplied by callers, so a caller cannot label an arbitrary host as allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Destination {
    /// A host on the content-free allowlist, or a subdomain of one.
    AllowlistedHost,
    /// A private, link-local or mDNS (`.local`) address.
    PrivateLan,
    /// Any other host. Only explicit per-meeting opt-ins (phase 6) may reach
    /// one, so the skeleton denies it under every policy.
    OtherHost,
}

impl NetPolicy {
    /// Whether this policy permits a connection to `host` (a DNS name or an IP
    /// literal, without scheme or port), given the content-free `allowlist`.
    pub fn permits(self, host: &str, allowlist: &[&str]) -> bool {
        match (self, classify(host, allowlist)) {
            (_, Destination::PrivateLan) => true,
            (NetPolicy::Default, Destination::AllowlistedHost) => true,
            (NetPolicy::StrictOffline, Destination::AllowlistedHost) => false,
            (_, Destination::OtherHost) => false,
        }
    }
}

fn classify(host: &str, allowlist: &[&str]) -> Destination {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return if is_private(ip) {
            Destination::PrivateLan
        } else {
            Destination::OtherHost
        };
    }
    if host.ends_with(".local") {
        return Destination::PrivateLan;
    }
    let allowed = allowlist.iter().any(|entry| {
        let entry = entry.to_ascii_lowercase();
        host == entry || host.ends_with(&format!(".{entry}"))
    });
    if allowed {
        Destination::AllowlistedHost
    } else {
        Destination::OtherHost
    }
}

/// RFC 1918 and link-local IPv4; unique-local (fc00::/7) and link-local
/// (fe80::/10) IPv6. Loopback is not LAN sync, so it is not included.
fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_private_v4(v4),
            None => is_private_v6(v6),
        },
    }
}

fn is_private_v4(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local()
}

fn is_private_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
}

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::NetPolicy::{self, *};

    const ALLOW: &[&str] = &["huggingface.co", "models.example.org"];

    #[test]
    fn default_policy_is_default() {
        assert_eq!(NetPolicy::default(), Default);
    }

    #[test]
    fn default_allows_allowlist_and_lan_only() {
        assert!(Default.permits("huggingface.co", ALLOW));
        assert!(Default.permits("cdn-lfs.huggingface.co", ALLOW));
        assert!(Default.permits("192.168.1.20", ALLOW));
        assert!(!Default.permits("api.openai.com", ALLOW));
        assert!(!Default.permits("evilhuggingface.co", ALLOW));
        assert!(!Default.permits("huggingface.co.evil.com", ALLOW));
    }

    #[test]
    fn strict_offline_allows_only_lan() {
        for lan in [
            "10.0.0.5",
            "172.16.3.4",
            "192.168.0.9",
            "169.254.1.1",
            "[fd12::1]",
            "fe80::1",
            "ghi-mac.local",
        ] {
            assert!(StrictOffline.permits(lan, ALLOW), "{lan}");
        }
        for internet in [
            "huggingface.co",
            "8.8.8.8",
            "2001:4860::8888",
            "172.32.0.1",
            "127.0.0.1",
        ] {
            assert!(!StrictOffline.permits(internet, ALLOW), "{internet}");
        }
    }

    #[test]
    fn ipv4_mapped_ipv6_is_classified_by_its_ipv4() {
        assert!(StrictOffline.permits("::ffff:192.168.1.2", ALLOW));
        assert!(!StrictOffline.permits("::ffff:8.8.8.8", ALLOW));
    }
}
