// SPDX-License-Identifier: Apache-2.0
//! EventKit on macOS (phase 14d): full access to events, read on demand. The
//! `EKEventStore` is created inside each blocking call (objc2 objects are not
//! `Send`). Notes are read only to find a call link and are never stored or
//! logged.
//!
//! W0-B stub: the signatures are final, the bodies are inert (slice S5).

// Nothing calls this until slice S5 fills in the commands.
#![allow(dead_code)]

use ghi_core::calendar::CalEvent;

/// What macOS says about calendar access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    NotDetermined,
    Denied,
    Authorized,
}

impl Access {
    /// The word `CalendarStatus.eventkit` carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Access::NotDetermined => "notDetermined",
            Access::Denied => "denied",
            Access::Authorized => "authorized",
        }
    }
}

/// The current authorization (never prompts).
pub fn access() -> Access {
    Access::NotDetermined
}

/// Shows the OS prompt (blocks until answered; call off the UI thread).
pub fn request_access() -> Access {
    Access::NotDetermined
}

/// Events starting inside `window` (unix ms, from..to), soonest first.
pub fn events(_window: (i64, i64)) -> Result<Vec<CalEvent>, String> {
    Err("notImplemented".into())
}
