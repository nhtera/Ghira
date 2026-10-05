// SPDX-License-Identifier: Apache-2.0
//! Conflict copies (slice 15-C2, doc 07 §7.5): the losing side of a concurrent
//! free-text edit, sealed under the meeting key, with a deterministic gid.

use super::not_yet;
use crate::Result;
use crate::store::Store;

/// A conflict copy, decrypted for the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct ConflictCopy {
    pub gid: String,
    pub meeting_gid: String,
    pub target_kind: String,
    pub target_gid: String,
    pub field: String,
    /// Device gid it was written on.
    pub origin: String,
    pub text: String,
}

impl Store {
    /// The open conflict copies of a meeting.
    pub fn conflict_copies(&self, _meeting_gid: &str) -> Result<Vec<ConflictCopy>> {
        not_yet("sync::conflicts::conflict_copies")
    }

    /// "Use this" (`use_it`) writes the copy's text into its target; either
    /// way the copy is tombstoned.
    pub fn resolve_conflict(&self, _gid: &str, _use_it: bool) -> Result<()> {
        not_yet("sync::conflicts::resolve_conflict")
    }
}
