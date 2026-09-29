// SPDX-License-Identifier: Apache-2.0
//! PCM ring buffers, resampling, echo cancellation and the Ogg/Opus writer.

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
