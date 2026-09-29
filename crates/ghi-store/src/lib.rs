// SPDX-License-Identifier: Apache-2.0
//! SQLCipher storage, Vietnamese-folded FTS5 search, audio bundles and key management.

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
