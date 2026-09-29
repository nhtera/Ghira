// SPDX-License-Identifier: Apache-2.0
//! Model registry, pinned downloads, SHA-256 checks and device tiers.

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
