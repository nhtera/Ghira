// SPDX-License-Identifier: Apache-2.0
//! The spoke (phone) side of a session.

use std::sync::Arc;

use super::SessionReport;
use crate::clock::Clock;
use crate::store::SyncStore;
use crate::transport::Transport;
use crate::{Result, not_yet};

/// One session towards the paired hub.
pub struct SpokeSession<T: Transport> {
    _store: Arc<dyn SyncStore>,
    _clock: Arc<dyn Clock>,
    _transport: T,
    _hub_device: String,
}

impl<T: Transport> SpokeSession<T> {
    pub fn new(
        store: Arc<dyn SyncStore>,
        clock: Arc<dyn Clock>,
        transport: T,
        hub_device: String,
    ) -> Self {
        Self {
            _store: store,
            _clock: clock,
            _transport: transport,
            _hub_device: hub_device,
        }
    }

    /// Runs one full pass (Hello through PullRows) and returns to idle.
    pub fn run_once(&mut self) -> Result<SessionReport> {
        not_yet("session::spoke::run_once")
    }

    /// Sends `Bye` (the app is going to the background) and ends the session.
    pub fn bye(&mut self) -> Result<()> {
        not_yet("session::spoke::bye")
    }
}
