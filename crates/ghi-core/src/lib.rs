// SPDX-License-Identifier: Apache-2.0
//! Session manager, job queue, aligner and pipeline orchestration.

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
