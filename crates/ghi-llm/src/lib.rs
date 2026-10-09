// SPDX-License-Identifier: Apache-2.0
//! The notes engine (doc 02 §D, §E, §G, §K): meeting notes, note enhancement
//! and "Ask this meeting" over a final transcript, with citations.
//!
//! - [`transcript`]: the input (segments with ids, times and speakers).
//! - [`provider`]: the model interface. The local model runs in the
//!   `ghi-llm-worker` process ([`sidecar`], [`providers::local`]); cloud
//!   providers are two-phase (prepare → send preview → confirmed send through
//!   `ghi-net` with a `CloudGrant`), see [`cloud`].
//! - [`template`]: note templates as data; the JSON schema and the GBNF grammar
//!   for constrained decoding are generated from them ([`schema`]).
//! - [`notes`] (map-reduce), [`enhance`], [`ask`]: the three tasks, each
//!   validated ([`validate`]) so every AI item cites segments that exist.
//! - [`redact`], [`preview`]: what a cloud request would carry, redacted and
//!   shown to the user before anything is sent.
//!
//! Model output is plain text plus citations; nothing here renders it.

pub mod ask;
pub mod cloud;
pub mod draft;
pub mod embed;
pub mod enhance;
pub mod local;
pub mod notes;
pub mod preview;
mod prompt;
pub mod provider;
pub mod redact;
pub mod retrieval;
pub mod run;
pub mod schema;
pub mod sidecar;
pub mod template;
pub mod transcript;
pub mod validate;

pub use provider::{Completion, EngineInfo, Llm, Message, Request, Role};
pub use transcript::{Segment, Transcript};

use std::fmt;

#[derive(Debug)]
pub enum LlmError {
    /// The worker process failed to start, crashed or broke the protocol.
    Worker(String),
    /// No reply within the time limit (the worker is killed).
    Timeout,
    /// The model output stayed invalid after the retries; `detail` names the
    /// first problem (never transcript text).
    InvalidOutput(String),
    /// A cloud provider answered with an error status.
    Provider {
        status: u16,
        message: String,
    },
    /// Refused by policy (no grant, cloud-locked or sensitive meeting, strict
    /// offline, preview not confirmed, ...).
    Denied(String),
    /// Transport failure talking to a cloud provider.
    Net(String),
    /// Bad caller input (unknown template, empty transcript, ...).
    Invalid(String),
    Io(std::io::Error),
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::Worker(d) => write!(f, "LLM worker: {d}"),
            LlmError::Timeout => f.write_str("LLM worker timed out"),
            LlmError::InvalidOutput(d) => write!(f, "model output invalid after retries: {d}"),
            LlmError::Provider { status, message } => {
                write!(f, "provider error {status}: {message}")
            }
            LlmError::Denied(d) => write!(f, "not allowed: {d}"),
            LlmError::Net(d) => write!(f, "network: {d}"),
            LlmError::Invalid(d) => write!(f, "invalid input: {d}"),
            LlmError::Io(e) => write!(f, "file: {e}"),
        }
    }
}

impl std::error::Error for LlmError {}

impl From<std::io::Error> for LlmError {
    fn from(e: std::io::Error) -> Self {
        LlmError::Io(e)
    }
}

pub type Result<T, E = LlmError> = std::result::Result<T, E>;

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
