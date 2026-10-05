// SPDX-License-Identifier: Apache-2.0
//! The hub (desktop) side of a session: answers one spoke's requests.

use std::sync::Arc;

use super::SessionReport;
use crate::clock::Clock;
use crate::store::SyncStore;
use crate::transport::Transport;
use crate::{Result, not_yet};

/// One session from an already authenticated spoke.
pub struct HubSession<T: Transport> {
    _store: Arc<dyn SyncStore>,
    _clock: Arc<dyn Clock>,
    _transport: T,
}

impl<T: Transport> HubSession<T> {
    pub fn new(store: Arc<dyn SyncStore>, clock: Arc<dyn Clock>, transport: T) -> Self {
        Self {
            _store: store,
            _clock: clock,
            _transport: transport,
        }
    }

    /// Answers requests until the spoke says `Bye`, the connection ends, or an
    /// idle or silence limit is hit.
    pub fn serve(&mut self) -> Result<SessionReport> {
        not_yet("session::hub::serve")
    }
}
