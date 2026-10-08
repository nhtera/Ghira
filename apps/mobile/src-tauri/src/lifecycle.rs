// SPDX-License-Identifier: Apache-2.0
//! The iOS app lifecycle, as the recording core and the job runner see it.
//!
//! ## Contract (wired by `platform.rs`, 16-E)
//!
//! | Swift                       | call                    | effect |
//! |-----------------------------|-------------------------|--------|
//! | `willResignActive`          | [`resign_active`]       | the engine gate closes: no new engine step (cheap; also fires for Control Center and banners) |
//! | `didEnterBackground`        | [`entered_background`]  | the gate learns whether a step overlapped the move (then the models reload and that audio is redone); `JobRunner::app_inactive` (jobs yield at their next checkpoint, nothing is claimed) |
//! | `didBecomeActive`           | [`become_active`]       | the gate opens (the engine catches up); `JobRunner::app_active` |
//! | thermal state notification  | [`thermal_changed`]     | serious or worse holds the engine; recording continues |
//! | audio interruption began    | [`interruption`]`(true)`| recording pauses, `MobileEvent::Interruption`; ended: a resume prompt, never an automatic resume |
//! | `CXCallObserver`            | [`call_active_changed`] | a call becoming active while recording is an interruption |
//! | Live Activity intents       | [`stop_requested`], [`mark_requested`] | |
//!
//! `JobRunner::app_active` is tied to `didBecomeActive`, not to
//! `willEnterForeground`: GPU work is allowed from the moment the app is
//! active, and the runner must not start a final pass before that.
//!
//! **Uninterruptible native calls** (the diarizer's `finish`, a 10 s block
//! push) cannot yield at a checkpoint. They run inside the engine gate
//! (`Gate::enter` .. `Gate::leave`, as every engine step does), so the gate
//! knows one overlapped the background move and drops its results. A job that
//! makes such a call off the engine thread must hold a `begin_bg_task` token
//! around it.
//!
//! **Launch in the background** (a Live Activity intent, a background
//! relaunch): the mobile core calls [`Lifecycle::launch`]`(true)` before it
//! creates the runner, and [`Lifecycle::sync_runner`] right after creating it
//! and *before* `JobRunner::spawn`, so no job is claimed while inactive.
//!
//! The Swift callbacks have no context of their own, so one [`Lifecycle`] is
//! installed process-wide ([`install`]); everything else is a method on it
//! (tests build their own).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use ghi_core::jobs::JobRunner;

use crate::cmd::events::{self as mobile_events, MobileEvent};
use crate::session;

type RunnerSlot = Arc<dyn Fn() -> Option<Arc<JobRunner>> + Send + Sync>;

/// A scene change the app reacts to besides the engine and the jobs (the app
/// lock and the privacy cover; `core.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// `willResignActive`.
    Resign,
    /// `didEnterBackground`.
    Background,
    /// `didBecomeActive`.
    Active,
}

type Observer = Arc<dyn Fn(Transition) + Send + Sync>;

pub struct Lifecycle {
    runner: RunnerSlot,
    /// Mirrors the app's activity so a runner created later starts right.
    active: AtomicBool,
    observer: std::sync::Mutex<Option<Observer>>,
}

static INSTALLED: OnceLock<Lifecycle> = OnceLock::new();

/// Installs the process-wide lifecycle (once; the mobile core does it with the
/// slot its job runner is put in).
pub fn install(runner: RunnerSlot) -> &'static Lifecycle {
    INSTALLED.get_or_init(|| Lifecycle::new(runner))
}

fn installed() -> Option<&'static Lifecycle> {
    INSTALLED.get()
}

impl Lifecycle {
    pub fn new(runner: RunnerSlot) -> Lifecycle {
        Lifecycle {
            runner,
            active: AtomicBool::new(true),
            observer: std::sync::Mutex::new(None),
        }
    }

    /// Sets the one observer of scene changes. It runs on the caller's thread
    /// (often the main thread) before anything else, so it must not block.
    pub fn observe(&self, f: Observer) {
        *self.observer.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    fn notify(&self, t: Transition) {
        let f = self
            .observer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(f) = f {
            f(t);
        }
    }

    /// The app launched in the background (`true`) or in the foreground.
    pub fn launch(&self, background: bool) {
        self.active.store(!background, Ordering::SeqCst);
    }

    /// Call right after creating the job runner and before spawning it.
    pub fn sync_runner(&self, runner: &JobRunner) {
        if !self.active.load(Ordering::SeqCst) {
            runner.app_inactive();
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// `willResignActive`.
    pub fn resign_active(&self) {
        self.notify(Transition::Resign);
        if let Some(s) = session::current() {
            s.shared.gate.suspend();
            s.activity_update();
        }
    }

    /// `didEnterBackground` (main thread, never blocks). `false`: an engine
    /// step overlapped the transition; the engine drops its results, reloads
    /// the models and redoes that audio.
    pub fn entered_background(&self) -> bool {
        log::info!("app entered the background");
        self.notify(Transition::Background);
        self.active.store(false, Ordering::SeqCst);
        if let Some(r) = (self.runner)() {
            r.app_inactive();
        }
        match session::current() {
            Some(s) => {
                let clean = s.shared.gate.entered_background();
                s.activity_update();
                clean
            }
            None => true,
        }
    }

    /// `didBecomeActive`.
    pub fn become_active(&self) {
        log::info!("app active");
        self.notify(Transition::Active);
        self.active.store(true, Ordering::SeqCst);
        if let Some(s) = session::current() {
            s.shared.gate.resume();
            s.activity_update();
        }
        if let Some(r) = (self.runner)() {
            r.app_active();
        }
    }

    /// `ProcessInfo.thermalState` changed (0 nominal .. 3 critical).
    pub fn thermal_changed(&self, state: i32) {
        log::info!("thermal state={state}");
        match session::current() {
            Some(s) => {
                s.shared.set_thermal(state);
                s.activity_update();
            }
            None => mobile_events::emit(MobileEvent::Thermal {
                level: state.clamp(0, 3) as u8,
            }),
        }
    }

    /// An audio interruption began or ended.
    pub fn interruption(&self, began: bool) {
        if let Some(s) = session::current()
            && !s.shared.capture_done()
        {
            s.interrupted(began);
        }
    }

    /// A phone call started or ended. Starting while recording interrupts it
    /// (the same path as the audio session interruption; iOS may deliver
    /// either first).
    pub fn call_active_changed(&self, active: bool) {
        mobile_events::emit(MobileEvent::CallActive { active });
        if let Some(s) = session::current()
            && !s.shared.capture_done()
        {
            if active {
                s.interrupted(true);
            } else {
                s.call_ended();
            }
        }
    }

    /// The system warned about memory. Inactive, the engine unloads its
    /// models now instead of waiting out the 30 s.
    pub fn memory_warning(&self) {
        mobile_events::emit(MobileEvent::MemoryWarning);
        if let Some(s) = session::current() {
            s.shared.gate.request_unload();
        }
    }

    /// The Live Activity's Stop.
    pub fn stop_requested(&self) {
        if let Some(s) = session::current() {
            let _ = s.stop();
        }
    }

    /// The Live Activity's Mark.
    pub fn mark_requested(&self) {
        if let Some(s) = session::current()
            && !s.shared.capture_done()
        {
            s.mark();
        }
    }
}

/// The process-wide entry points the C ABI calls; no-ops before [`install`].
pub fn resign_active() {
    if let Some(l) = installed() {
        l.resign_active();
    }
}

pub fn entered_background() -> bool {
    installed().is_none_or(Lifecycle::entered_background)
}

pub fn become_active() {
    if let Some(l) = installed() {
        l.become_active();
    }
}

pub fn thermal_changed(state: i32) {
    if let Some(l) = installed() {
        l.thermal_changed(state);
    }
}

pub fn interruption(began: bool) {
    if let Some(l) = installed() {
        l.interruption(began);
    }
}

pub fn call_active_changed(active: bool) {
    if let Some(l) = installed() {
        l.call_active_changed(active);
    }
}

pub fn memory_warning() {
    match installed() {
        Some(l) => l.memory_warning(),
        None => mobile_events::emit(MobileEvent::MemoryWarning),
    }
}

/// Whether the app is active (true before [`install`]).
pub fn app_active() -> bool {
    installed().is_none_or(Lifecycle::is_active)
}

pub fn stop_requested() {
    if let Some(l) = installed() {
        l.stop_requested();
    }
}

pub fn mark_requested() {
    if let Some(l) = installed() {
        l.mark_requested();
    }
}

#[cfg(test)]
mod observer_tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn the_observer_sees_each_scene_change_before_the_engine_work() {
        let l = Lifecycle::new(Arc::new(|| None));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        l.observe(Arc::new(move |t| s.lock().unwrap().push(t)));
        l.resign_active();
        assert!(l.entered_background());
        assert!(!l.is_active());
        l.become_active();
        assert!(l.is_active());
        assert_eq!(
            *seen.lock().unwrap(),
            [
                Transition::Resign,
                Transition::Background,
                Transition::Active
            ]
        );
    }
}
