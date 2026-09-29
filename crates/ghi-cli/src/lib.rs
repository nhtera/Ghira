// SPDX-License-Identifier: Apache-2.0
//! Headless CLI used by the eval harness and tests.

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
