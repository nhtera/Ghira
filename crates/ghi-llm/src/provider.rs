// SPDX-License-Identifier: Apache-2.0
//! The model interface shared by the local worker and cloud providers.

use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Message {
        Message {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Message {
        Message {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// One structured completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub messages: Vec<Message>,
    /// JSON Schema of the expected output: cloud JSON modes, and constrained
    /// decoding in the local worker (converted to a llama.cpp grammar there).
    pub schema: Option<serde_json::Value>,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    pub text: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    /// Generation stopped at `max_tokens` (the JSON is likely cut off).
    pub truncated: bool,
}

/// Which model produced the output (`ghi.notes/1` `engine`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub name: String,
    pub version: String,
}

/// A model that answers requests directly: the local worker, or a fake in
/// tests. Cloud providers are not `Llm`s: every send needs a confirmed
/// preview (see `cloud`).
pub trait Llm {
    fn engine(&self) -> EngineInfo;
    /// Tokens the model can attend to (prompt + output); chunking uses it.
    fn context_tokens(&self) -> u32;
    fn complete(&mut self, req: &Request) -> Result<Completion>;
    /// Tokens `text` takes as message content. The default is a high
    /// estimate; the local model counts exactly.
    fn count_tokens(&mut self, text: &str) -> Result<u32> {
        Ok(crate::run::estimate_tokens(text))
    }
}
