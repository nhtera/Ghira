// SPDX-License-Identifier: Apache-2.0
//! Which addresses a cloud request may connect to.
//!
//! A provider host must resolve to public addresses only, so a hostile or
//! rebound DNS answer cannot steer a request (and the API key in it) to the
//! loopback, the LAN or a cloud metadata endpoint (169.254.169.254).
//! Loopback is always denied: v1 has no localhost Ollama-style provider.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// True for globally routable unicast addresses. Everything special-purpose is
/// denied, and IPv6 forms that embed an IPv4 address (mapped, compatible,
/// NAT64, 6to4, Teredo) are judged by the IPv4 inside them.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

/// Private LAN ranges (RFC 1918, unique-local): the only non-public addresses
/// a LAN-host grant may reach. Never loopback, and not link-local either
/// (169.254.169.254 is the cloud metadata endpoint).
pub fn is_lan(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_lan_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_lan_v4(v4),
            None => (v6.segments()[0] & 0xfe00) == 0xfc00,
        },
    }
}

fn is_lan_v4(ip: Ipv4Addr) -> bool {
    ip.is_private()
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0 // 0/8 "this network"
        || a == 10
        || a == 127
        || (a == 100 && (b & 0xc0) == 64) // 100.64/10 CGNAT
        || (a == 169 && b == 254)
        || (a == 172 && (b & 0xf0) == 16)
        || (a == 192 && b == 0 && c == 0) // 192.0.0/24 IETF protocol
        || (a == 192 && b == 0 && c == 2) // TEST-NET-1
        || (a == 192 && b == 88 && c == 99) // 6to4 relay anycast (deprecated)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19)) // benchmarking
        || (a == 198 && b == 51 && c == 100) // TEST-NET-2
        || (a == 203 && b == 0 && c == 113) // TEST-NET-3
        || a >= 224) // multicast 224/4, reserved 240/4, broadcast
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    if ip.is_unspecified() || ip.is_loopback() {
        return false;
    }
    // ::ffff:0:0/96 mapped.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    // ::/96 IPv4-compatible (deprecated), judged by the IPv4.
    if s[..6].iter().all(|&x| x == 0) {
        return is_public_v4(v4_of(s[6], s[7]));
    }
    // 64:ff9b::/96 NAT64; 64:ff9b:1::/48 is local-use, never public.
    if s[0] == 0x64 && s[1] == 0xff9b {
        return s[2] == 0 && s[3..6].iter().all(|&x| x == 0) && is_public_v4(v4_of(s[6], s[7]));
    }
    // 2002::/16 6to4: the IPv4 is in segments 1-2.
    if s[0] == 0x2002 {
        return is_public_v4(v4_of(s[1], s[2]));
    }
    // 2001::/32 Teredo: server IPv4 in segments 2-3, client IPv4 (bitwise
    // inverted) in segments 6-7; both must be public.
    if s[0] == 0x2001 && s[1] == 0 {
        return is_public_v4(v4_of(s[2], s[3])) && is_public_v4(v4_of(!s[6], !s[7]));
    }
    // Only global unicast (2000::/3) is public; everything else (unique- and
    // site-local, link-local, multicast, 100::/64 discard, 5f00::/16 SRv6 ...)
    // is not. Within it, special-purpose blocks are excluded too.
    if (s[0] & 0xe000) != 0x2000 {
        return false;
    }
    !((s[0] == 0x2001 && s[1] == 0x0db8) // 2001:db8::/32 documentation
        || (s[0] == 0x2001 && s[1] == 0x0002 && s[2] == 0) // 2001:2::/48 benchmarking
        || (s[0] == 0x2001 && (s[1] & 0xffe0) == 0x0020) // 2001:20::/28 ORCHIDv2
        || (s[0] == 0x2001 && (s[1] & 0xfff0) == 0x0010) // 2001:10::/28 ORCHID
        || (s[0] & 0xfff0) == 0x3ff0) // 3fff::/20 documentation (3ff0::/12 to be safe)
}

fn v4_of(hi: u16, lo: u16) -> Ipv4Addr {
    let [a, b] = hi.to_be_bytes();
    let [c, d] = lo.to_be_bytes();
    Ipv4Addr::new(a, b, c, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn special_ipv6_and_relay_ranges_are_not_public() {
        for a in [
            "fec0::1",
            "3fff::1",
            "100::1",
            "2001:2::1",
            "5f00::1",
            "2001:20::1",
            "192.88.99.1",
        ] {
            assert!(!is_public(ip(a)), "{a}");
        }
        for a in ["2606:4700::1111", "2a00:1450:4001::1", "1.1.1.1"] {
            assert!(is_public(ip(a)), "{a}");
        }
    }

    #[test]
    fn denies_special_purpose_ipv4() {
        for s in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "10.255.255.255",
            "127.0.0.1",
            "127.255.0.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "100.64.0.1",
            "100.127.255.255",
            "169.254.169.254",
            "192.0.0.8",
            "192.0.2.5",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!is_public(ip(s)), "{s} must be denied");
        }
    }

    #[test]
    fn allows_public_ipv4_next_to_denied_ranges() {
        for s in [
            "8.8.8.8",
            "1.1.1.1",
            "172.15.255.255",
            "172.32.0.1",
            "100.63.255.255",
            "100.128.0.1",
            "198.17.0.1",
            "198.20.0.1",
            "192.0.1.1",
            "192.169.0.1",
            "203.0.112.1",
            "223.255.255.255",
            "104.18.7.192",
        ] {
            assert!(is_public(ip(s)), "{s} must be allowed");
        }
    }

    #[test]
    fn denies_special_purpose_ipv6() {
        for s in [
            "::",
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "febf::1",
            "ff02::1",
            "2001:db8::1",
            "2001:db8:ffff::1",
            "64:ff9b:1::1",
        ] {
            assert!(!is_public(ip(s)), "{s} must be denied");
        }
    }

    #[test]
    fn allows_public_ipv6() {
        for s in [
            "2001:4860:4860::8888",
            "2606:4700:4700::1111",
            "2a00:1450::1",
        ] {
            assert!(is_public(ip(s)), "{s} must be allowed");
        }
    }

    #[test]
    fn embedded_ipv4_is_judged_by_its_ipv4() {
        // IPv4-mapped.
        assert!(!is_public(ip("::ffff:127.0.0.1")));
        assert!(!is_public(ip("::ffff:10.1.2.3")));
        assert!(!is_public(ip("::ffff:169.254.169.254")));
        assert!(is_public(ip("::ffff:8.8.8.8")));
        // IPv4-compatible.
        assert!(!is_public(ip("::192.168.0.1")));
        assert!(is_public(ip("::8.8.8.8")));
        // NAT64.
        assert!(!is_public(ip("64:ff9b::7f00:1")));
        assert!(!is_public(ip("64:ff9b::a00:1")));
        assert!(is_public(ip("64:ff9b::808:808")));
        // 6to4 (2002:AABB:CCDD::/48 embeds AA.BB.CC.DD).
        assert!(!is_public(ip("2002:7f00:1::1")));
        assert!(!is_public(ip("2002:c0a8:101::1")));
        assert!(is_public(ip("2002:808:808::1")));
        // Teredo: server 8.8.8.8; client 192.168.1.1 is stored inverted.
        assert!(!is_public(ip("2001:0:808:808::3f57:fefe")));
        // Teredo with a public client (inverted 8.8.4.4 = f7f7:fbfb) and a
        // public server.
        assert!(is_public(ip("2001:0:808:808::f7f7:fbfb")));
        // Teredo with a private server.
        assert!(!is_public(ip("2001:0:a00:1::f7f7:fbfb")));
    }

    #[test]
    fn lan_ranges_never_include_loopback() {
        for s in [
            "10.0.0.5",
            "172.16.3.4",
            "192.168.0.9",
            "fd12::1",
            "::ffff:192.168.1.2",
        ] {
            assert!(is_lan(ip(s)), "{s}");
        }
        for s in [
            "127.0.0.1",
            "::1",
            "8.8.8.8",
            "0.0.0.0",
            "224.0.0.1",
            "100.64.0.1",
            "2001:db8::1",
            "169.254.169.254",
            "fe80::1",
        ] {
            assert!(!is_lan(ip(s)), "{s}");
        }
    }
}
