// SPDX-License-Identifier: Apache-2.0
//! The clock leases are measured with (doc 07 §8, decision D9).
//!
//! Lease deadlines are durations on a **sleep-inclusive** monotonic clock:
//! `mach_continuous_time` on Apple (`Instant` stops during sleep on macOS and
//! must not be used) and `QueryInterruptTime` on Windows. No device compares
//! its clock with a peer's. A [`Clock::boot_id`] names the boot a reading
//! belongs to: after a reboot the readings restart, so a lease measured in
//! another boot is suspended until renewed.
//!
//! Tests inject [`FakeClock`].

use std::sync::Mutex;
use std::time::Duration;

/// A sleep-inclusive monotonic clock plus the wall clock (display and grace
/// periods only).
pub trait Clock: Send + Sync {
    /// Nanoseconds on the sleep-inclusive monotonic clock. Only differences
    /// within one [`Clock::boot_id`] mean anything.
    fn now_cont_ns(&self) -> u64;
    /// Names the boot [`Clock::now_cont_ns`] counts from.
    fn boot_id(&self) -> String;
    /// Unix milliseconds. Never compared with a peer's.
    fn wall_ms(&self) -> i64;
}

/// The real clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_cont_ns(&self) -> u64 {
        sys::now_cont_ns()
    }

    fn boot_id(&self) -> String {
        sys::boot_id()
    }

    fn wall_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }
}

#[cfg(target_vendor = "apple")]
mod sys {
    use std::sync::OnceLock;

    #[repr(C)]
    struct TimebaseInfo {
        numer: u32,
        denom: u32,
    }

    unsafe extern "C" {
        fn mach_continuous_time() -> u64;
        fn mach_timebase_info(info: *mut TimebaseInfo) -> i32;
    }

    /// Ticks to nanoseconds: `(numer, denom)`; 1:1 if the call fails.
    fn timebase() -> (u128, u128) {
        static TB: OnceLock<(u128, u128)> = OnceLock::new();
        *TB.get_or_init(|| {
            let mut info = TimebaseInfo { numer: 0, denom: 0 };
            // SAFETY: `info` is a valid out-pointer for the call.
            let rc = unsafe { mach_timebase_info(&mut info) };
            if rc != 0 || info.denom == 0 {
                (1, 1)
            } else {
                (u128::from(info.numer), u128::from(info.denom))
            }
        })
    }

    pub fn now_cont_ns() -> u64 {
        // SAFETY: no arguments, no preconditions.
        let ticks = unsafe { mach_continuous_time() };
        let (numer, denom) = timebase();
        // Widen: ticks * numer overflows u64 within days on Apple silicon (125/3).
        (u128::from(ticks) * numer / denom) as u64
    }

    /// `kern.boottime` (seconds and microseconds since the epoch).
    pub fn boot_id() -> String {
        let mut tv = libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        };
        let mut len = std::mem::size_of::<libc::timeval>();
        // SAFETY: the name is NUL-terminated, `tv` and `len` are valid for the call.
        let rc = unsafe {
            libc::sysctlbyname(
                c"kern.boottime".as_ptr(),
                (&mut tv as *mut libc::timeval).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 {
            return "boot-unknown".to_string();
        }
        format!("{}.{:06}", tv.tv_sec, tv.tv_usec)
    }
}

#[cfg(windows)]
mod sys {
    use windows_sys::Win32::System::WindowsProgramming::QueryInterruptTime;

    /// 100 ns units, including time asleep.
    pub fn now_cont_ns() -> u64 {
        let mut t: u64 = 0;
        // SAFETY: `t` is a valid out-pointer for the call.
        unsafe { QueryInterruptTime(&mut t) };
        t.saturating_mul(100)
    }

    /// Not verified on Windows yet (phase 13): a constant means a reboot is
    /// not noticed, so a lease may outlive one (fencing still keeps one result).
    pub fn boot_id() -> String {
        "windows".to_string()
    }
}

#[cfg(not(any(target_vendor = "apple", windows)))]
mod sys {
    use std::sync::OnceLock;
    use std::time::Instant;

    static START: OnceLock<Instant> = OnceLock::new();

    /// Not sleep-inclusive: Ghira ships on Apple and Windows only. For CI on
    /// other hosts.
    pub fn now_cont_ns() -> u64 {
        let start = START.get_or_init(Instant::now);
        u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    pub fn boot_id() -> String {
        "process".to_string()
    }
}

/// A clock a test moves by hand.
#[derive(Debug)]
pub struct FakeClock {
    state: Mutex<FakeState>,
}

#[derive(Debug)]
struct FakeState {
    cont_ns: u64,
    wall_ms: i64,
    boot: u32,
}

impl FakeClock {
    /// Starts at 1 s of uptime, wall time `wall_ms`, boot 1.
    pub fn new(wall_ms: i64) -> Self {
        Self {
            state: Mutex::new(FakeState {
                cont_ns: 1_000_000_000,
                wall_ms,
                boot: 1,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Time passes, asleep or not: both clocks move.
    pub fn advance(&self, by: Duration) {
        let mut s = self.state();
        s.cont_ns += u64::try_from(by.as_nanos()).unwrap_or(u64::MAX);
        s.wall_ms += i64::try_from(by.as_millis()).unwrap_or(i64::MAX);
    }

    /// The user changes the wall clock; the monotonic one doesn't move.
    pub fn set_wall_ms(&self, wall_ms: i64) {
        self.state().wall_ms = wall_ms;
    }

    /// A reboot: a new boot id, and the monotonic clock starts over.
    pub fn reboot(&self) {
        let mut s = self.state();
        s.boot += 1;
        s.cont_ns = 1_000_000_000;
    }
}

impl Clock for FakeClock {
    fn now_cont_ns(&self) -> u64 {
        self.state().cont_ns
    }

    fn boot_id(&self) -> String {
        format!("fake-boot-{}", self.state().boot)
    }

    fn wall_ms(&self) -> i64 {
        self.state().wall_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clock_advances_both_clocks() {
        let c = FakeClock::new(1_000);
        let (t0, w0) = (c.now_cont_ns(), c.wall_ms());
        c.advance(Duration::from_secs(121));
        assert_eq!(c.now_cont_ns() - t0, 121_000_000_000);
        assert_eq!(c.wall_ms() - w0, 121_000);
    }

    #[test]
    fn wall_clock_changes_do_not_move_the_monotonic_clock() {
        let c = FakeClock::new(1_000);
        let t0 = c.now_cont_ns();
        c.set_wall_ms(5);
        assert_eq!((c.now_cont_ns(), c.wall_ms()), (t0, 5));
    }

    #[test]
    fn reboot_changes_the_boot_id_and_restarts_the_clock() {
        let c = FakeClock::new(0);
        let boot = c.boot_id();
        c.advance(Duration::from_secs(10));
        c.reboot();
        assert_ne!(c.boot_id(), boot);
        assert_eq!(c.now_cont_ns(), 1_000_000_000);
    }

    #[test]
    fn the_system_clock_is_monotonic_and_has_a_boot_id() {
        let c = SystemClock;
        let a = c.now_cont_ns();
        let b = c.now_cont_ns();
        assert!(b >= a);
        assert!(!c.boot_id().is_empty());
        assert!(c.wall_ms() > 1_700_000_000_000);
    }
}
