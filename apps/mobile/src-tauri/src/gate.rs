// SPDX-License-Identifier: Apache-2.0
//! When the live engine may touch the GPU (RT-5).
//!
//! iOS forbids GPU command submission from a backgrounded app, and a refused
//! submission is silent (ggml only logs it). So:
//! - on resign-active, [`Gate::suspend`] stops new engine steps (cheap; it
//!   also fires for Control Center and banners);
//! - on entering the background, [`Gate::entered_background`] checks whether a
//!   step (or the model load) was in flight at resign-active or still is. If
//!   so, some of its GPU work may have been refused: the gate is *poisoned*
//!   and the engine reloads the models ([`Gate::take_poison`]).
//! - a step that returns while the app is not active is *suspect*
//!   ([`Gate::leave`]): the engine drops its results, reopens its streams and
//!   redoes that audio from the backlog.
//!
//! A hot phone (`.serious` thermal state or worse) holds the engine too.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
struct State {
    /// The app is (about to be) in the background.
    suspended: bool,
    /// Thermal state is `.serious` or `.critical`.
    hot: bool,
    /// The engine is inside a step.
    busy: bool,
    /// The session ends once the backlog is drained.
    stopping: bool,
    /// A step was in flight when the app resigned active.
    busy_at_suspend: bool,
    /// A step overlapped the move to the background; not yet handled.
    poisoned: bool,
    /// How often that happened (metrics).
    overlaps: u32,
    /// Since when the app has been inactive (the models unload after 30 s).
    suspended_at: Option<Instant>,
    /// Engine steps that ended while the app was not active (metrics).
    inactive_steps: u32,
    /// A memory warning arrived while inactive: unload now, not after 30 s.
    unload_now: bool,
}

#[derive(Debug, Default)]
pub struct Gate {
    state: Mutex<State>,
    cv: Condvar,
}

/// Why the engine is held, for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    None,
    Suspended,
    Hot,
}

impl Gate {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The app will resign active: no new engine steps.
    pub fn suspend(&self) {
        let mut s = self.lock();
        if !s.suspended {
            s.busy_at_suspend = s.busy;
            s.suspended_at = Some(Instant::now());
        }
        s.suspended = true;
    }

    /// The app is in the background (called on the main thread, before any
    /// later resume; never blocks). Poisons the gate if a step was in flight
    /// at resign-active or still is: it may have submitted GPU work after the
    /// transition. Returns `true` if no step overlapped.
    pub fn entered_background(&self) -> bool {
        let mut s = self.lock();
        if !s.suspended {
            s.suspended_at = Some(Instant::now());
        }
        s.suspended = true;
        let overlapped = s.busy || s.busy_at_suspend;
        if overlapped {
            s.poisoned = true;
            s.overlaps += 1;
        }
        s.busy_at_suspend = false;
        !overlapped
    }

    /// Engine side: whether a step overlapped a move to the background since
    /// the last call (the models and streams must be rebuilt).
    #[cfg_attr(not(any(test, feature = "nemo")), allow(dead_code))]
    pub fn take_poison(&self) -> bool {
        std::mem::take(&mut self.lock().poisoned)
    }

    /// Steps that overlapped a move to the background so far.
    pub fn overlaps(&self) -> u32 {
        self.lock().overlaps
    }

    /// How long the app has been inactive, if it is.
    pub fn suspended_for(&self) -> Option<Duration> {
        self.lock().suspended_at.map(|t| t.elapsed())
    }

    /// The models should go: inactive for the grace period, or the system
    /// warned about memory while inactive.
    pub fn unload_due(&self, after: Duration) -> bool {
        let s = self.lock();
        s.suspended && (s.unload_now || s.suspended_at.is_some_and(|t| t.elapsed() >= after))
    }

    /// A memory warning: unload at once if the app is not active (the models
    /// are what jetsam would take the app for).
    pub fn request_unload(&self) {
        let mut s = self.lock();
        if s.suspended {
            s.unload_now = true;
        }
        drop(s);
        self.cv.notify_all();
    }

    /// Engine steps that ended while the app was not active.
    pub fn steps_while_inactive(&self) -> u32 {
        self.lock().inactive_steps
    }

    /// Makes the gate look inactive since `ago` (tests of the 30 s unload).
    #[cfg(test)]
    pub fn pretend_suspended_for(&self, ago: Duration) {
        self.lock().suspended_at = Instant::now().checked_sub(ago);
    }

    pub fn resume(&self) {
        let mut s = self.lock();
        s.suspended = false;
        s.suspended_at = None;
        s.unload_now = false;
        s.busy_at_suspend = false;
        drop(s);
        self.cv.notify_all();
    }

    pub fn set_hot(&self, hot: bool) {
        self.lock().hot = hot;
        self.cv.notify_all();
    }

    pub fn stop(&self) {
        self.lock().stopping = true;
        self.cv.notify_all();
    }

    pub fn hold(&self) -> Hold {
        let s = self.lock();
        if s.suspended {
            Hold::Suspended
        } else if s.hot {
            Hold::Hot
        } else {
            Hold::None
        }
    }

    pub fn stopping(&self) -> bool {
        self.lock().stopping
    }

    /// Engine side: waits until it may run, then marks a step as in flight.
    /// Wakes at least every `poll` so the caller can look for new audio.
    pub fn enter(&self, poll: Duration) -> bool {
        let s = self.lock();
        let (mut s, _) = self
            .cv
            .wait_timeout_while(s, poll, |s| s.suspended || s.hot)
            .unwrap_or_else(|e| e.into_inner());
        if s.suspended || s.hot {
            return false;
        }
        s.busy = true;
        true
    }

    /// Engine side: the step has returned; no GPU work is in flight.
    /// Returns `true` if its results are suspect (it ended while the app was
    /// not active, or overlapped a move to the background): drop them.
    pub fn leave(&self) -> bool {
        let mut s = self.lock();
        s.busy = false;
        let suspect = s.poisoned || s.suspended;
        if s.suspended {
            s.inactive_steps += 1;
        }
        drop(s);
        self.cv.notify_all();
        suspect
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    const POLL: Duration = Duration::from_millis(5);

    #[test]
    fn a_step_in_flight_at_resign_poisons_even_if_done_before_background() {
        let gate = Gate::default();
        assert!(gate.enter(POLL));
        gate.suspend();
        assert!(gate.leave(), "ended while not active: suspect");
        assert!(!gate.entered_background(), "overlapped");
        assert!(!gate.enter(POLL), "held while suspended");
        assert_eq!(gate.hold(), Hold::Suspended);
        gate.resume();
        assert!(gate.take_poison());
        assert!(!gate.take_poison(), "taken once");
        assert_eq!(gate.overlaps(), 1);
        assert!(gate.enter(POLL));
        assert!(!gate.leave());
    }

    #[test]
    fn a_step_still_running_at_background_learns_it_on_leave() {
        let gate = Arc::new(Gate::default());
        assert!(gate.enter(POLL));
        gate.suspend();
        let g = gate.clone();
        let engine = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            g.leave()
        });
        assert!(!gate.entered_background());
        assert!(engine.join().unwrap(), "leave reports the poison");
        let flag = AtomicBool::new(gate.take_poison());
        assert!(flag.load(Ordering::SeqCst));
    }

    #[test]
    fn an_idle_engine_is_not_poisoned_and_heat_holds() {
        let gate = Gate::default();
        gate.suspend();
        assert!(!gate.enter(POLL));
        assert!(gate.entered_background());
        gate.resume();
        assert!(!gate.take_poison());
        gate.set_hot(true);
        assert!(!gate.enter(POLL));
        assert_eq!(gate.hold(), Hold::Hot);
        gate.set_hot(false);
        assert!(gate.enter(POLL));
        assert!(!gate.leave());
        assert!(!gate.stopping());
        gate.stop();
        assert!(gate.stopping());
    }

    #[test]
    fn a_memory_warning_while_inactive_unloads_at_once() {
        let gate = Gate::default();
        gate.request_unload();
        assert!(!gate.unload_due(Duration::from_secs(30)), "active: nothing");
        gate.suspend();
        assert!(!gate.unload_due(Duration::from_secs(30)), "grace period");
        gate.request_unload();
        assert!(gate.unload_due(Duration::from_secs(30)));
        gate.resume();
        assert!(!gate.unload_due(Duration::from_secs(30)));
    }
}
