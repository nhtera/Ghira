// SPDX-License-Identifier: Apache-2.0
//! Session manager, job queue, aligner and pipeline orchestration.

pub mod aligner;
pub mod capture;
pub mod carry;
pub mod cloud;
pub mod diff;
pub mod email;
pub mod engines;
pub mod events;
pub mod export;
pub mod final_pass;
pub mod import;
pub mod index_job;
pub mod jobs;
pub mod live;
pub mod notes_job;
pub mod pages;
pub mod persist;
pub mod recover;
pub mod session;
pub mod speakers;
pub mod vocab;

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
