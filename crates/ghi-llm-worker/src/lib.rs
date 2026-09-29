// SPDX-License-Identifier: Apache-2.0
//! llama.cpp worker run as a separate process, spoken to over stdio.

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
