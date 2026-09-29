// SPDX-License-Identifier: Apache-2.0
//! Client for the LLM worker process, prompts, JSON schemas, cloud providers and redaction.

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
