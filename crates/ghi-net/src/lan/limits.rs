// SPDX-License-Identifier: Apache-2.0
//! Abuse limits of the hub's listener (doc 07 §5.3): at most
//! [`MAX_UNKNOWN_PER_MINUTE`] failed (unknown-key) handshakes per source IP
//! per minute, then the IP is dropped for [`BAN`]; at most [`MAX_SESSIONS`]
//! concurrent sessions. The listener enforces them in `accept`; the session
//! layer feeds the first through [`Limits::report_handshake_failure`].

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Unknown-key handshakes allowed per IP per [`WINDOW`].
pub const MAX_UNKNOWN_PER_MINUTE: usize = 10;
pub const WINDOW: Duration = Duration::from_secs(60);
/// How long an IP that went over is dropped.
pub const BAN: Duration = Duration::from_secs(5 * 60);
/// Concurrent sessions the hub serves.
pub const MAX_SESSIONS: usize = 4;

#[derive(Debug, Default)]
struct PerIp {
    failures: VecDeque<Instant>,
    banned_until: Option<Instant>,
}

/// Shared by the listener and the session layer (clone the `Arc`).
#[derive(Debug, Default)]
pub struct Limits {
    ips: Mutex<HashMap<IpAddr, PerIp>>,
    sessions: AtomicUsize,
}

/// Holds one of the [`MAX_SESSIONS`] slots; frees it on drop.
#[derive(Debug)]
pub struct SessionSlot {
    limits: Arc<Limits>,
}

impl Drop for SessionSlot {
    fn drop(&mut self) {
        self.limits.sessions.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Limits {
    pub fn new() -> Arc<Limits> {
        Arc::new(Limits::default())
    }

    /// Call when a handshake from `ip` failed (unknown key, wrong PSK, junk).
    pub fn report_handshake_failure(&self, ip: IpAddr) {
        self.report_at(ip, Instant::now());
    }

    /// True while `ip` is dropped.
    pub fn is_banned(&self, ip: IpAddr) -> bool {
        self.is_banned_at(ip, Instant::now())
    }

    pub(crate) fn report_at(&self, ip: IpAddr, now: Instant) {
        let mut ips = self.ips.lock().unwrap_or_else(|p| p.into_inner());
        // Forget idle entries so a scan of the subnet cannot grow the map.
        ips.retain(|_, e| {
            e.banned_until.is_some_and(|t| t > now)
                || e.failures
                    .back()
                    .is_some_and(|t| now.duration_since(*t) < WINDOW)
        });
        let e = ips.entry(ip).or_default();
        while e
            .failures
            .front()
            .is_some_and(|t| now.duration_since(*t) >= WINDOW)
        {
            e.failures.pop_front();
        }
        e.failures.push_back(now);
        if e.failures.len() >= MAX_UNKNOWN_PER_MINUTE {
            e.banned_until = Some(now + BAN);
            e.failures.clear();
        }
    }

    pub(crate) fn is_banned_at(&self, ip: IpAddr, now: Instant) -> bool {
        let ips = self.ips.lock().unwrap_or_else(|p| p.into_inner());
        ips.get(&ip)
            .and_then(|e| e.banned_until)
            .is_some_and(|t| t > now)
    }

    /// A free session slot, or `None` at the cap.
    pub(crate) fn try_slot(self: &Arc<Self>) -> Option<SessionSlot> {
        let mut cur = self.sessions.load(Ordering::SeqCst);
        loop {
            if cur >= MAX_SESSIONS {
                return None;
            }
            match self
                .sessions
                .compare_exchange(cur, cur + 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    return Some(SessionSlot {
                        limits: Arc::clone(self),
                    });
                }
                Err(now) => cur = now,
            }
        }
    }

    /// Sessions open right now.
    pub fn sessions(&self) -> usize {
        self.sessions.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn the_tenth_failure_in_a_minute_bans_for_five() {
        let l = Limits::new();
        let t0 = Instant::now();
        let a = ip("192.168.1.9");
        for i in 0..9 {
            l.report_at(a, t0 + Duration::from_secs(i));
            assert!(!l.is_banned_at(a, t0 + Duration::from_secs(i)));
        }
        l.report_at(a, t0 + Duration::from_secs(9));
        assert!(l.is_banned_at(a, t0 + Duration::from_secs(10)));
        assert!(l.is_banned_at(a, t0 + Duration::from_secs(9 + 299)));
        assert!(!l.is_banned_at(a, t0 + Duration::from_secs(9 + 301)));
        // Other addresses are unaffected.
        assert!(!l.is_banned_at(ip("192.168.1.10"), t0 + Duration::from_secs(10)));
    }

    #[test]
    fn slow_failures_never_ban() {
        let l = Limits::new();
        let t0 = Instant::now();
        let a = ip("10.0.0.2");
        for i in 0..40 {
            l.report_at(a, t0 + Duration::from_secs(i * 10));
        }
        assert!(!l.is_banned_at(a, t0 + Duration::from_secs(400)));
    }

    #[test]
    fn at_most_four_sessions() {
        let l = Limits::new();
        let slots: Vec<_> = (0..MAX_SESSIONS).map(|_| l.try_slot().unwrap()).collect();
        assert!(l.try_slot().is_none());
        assert_eq!(l.sessions(), MAX_SESSIONS);
        drop(slots);
        assert_eq!(l.sessions(), 0);
        assert!(l.try_slot().is_some());
    }
}
