// SPDX-License-Identifier: Apache-2.0
//! llama.cpp worker run as a separate process, spoken to over stdio.
//!
//! The protocol is one JSON object per line each way. Every request carries an
//! `id` that the reply echoes. Replies never contain prompt or output text
//! other than the completion itself (errors carry a short message only).
//!
//! The worker's first line, before any request, is a [`Hello`]; the parent
//! refuses a worker whose protocol number differs from its own.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Protocol revision; bump on any incompatible change to the lines below.
///
/// 3 added the embedding ops (`load_embed`, `embed`); the notes ops are
/// unchanged from 2.
pub const PROTOCOL: u32 = 3;

/// First line the worker writes: `{"kind":"hello","protocol":1,"worker":"0.1.0"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub kind: String,
    pub protocol: u32,
    /// Crate version of the worker binary.
    pub worker: String,
}

impl Hello {
    pub fn current() -> Hello {
        Hello {
            kind: "hello".into(),
            protocol: PROTOCOL,
            worker: version().into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireMessage {
    /// `system`, `user` or `assistant`.
    pub role: String,
    pub content: String,
}

/// A request line: `{"id":1,"op":"load",...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub op: Op,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Load {
        model_path: String,
        n_ctx: u32,
        n_gpu_layers: u32,
        seed: u32,
        /// Prompt template family; only `qwen3` is known.
        chat_format: String,
    },
    Complete {
        messages: Vec<WireMessage>,
        schema: Option<Value>,
        max_tokens: u32,
        temperature: f32,
    },
    /// Token count of `text` as message content (special tokens not parsed).
    Count {
        text: String,
    },
    /// Load an embedding model (a worker serves either the notes LLM or an
    /// embedder, never both). Last-token pooling, L2-normalized output.
    LoadEmbed {
        model_path: String,
        /// Longest text embedded, in tokens; longer texts are cut.
        max_tokens: u32,
        n_gpu_layers: u32,
    },
    /// One vector per text, in order. The texts are embedded exactly as given
    /// (the caller adds any query instruction).
    Embed {
        texts: Vec<String>,
    },
    Health,
    Shutdown,
}

/// A reply line: `{"id":1,"kind":"loaded",...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub id: u64,
    #[serde(flatten)]
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Loaded {
        n_ctx: u32,
        load_s: f64,
    },
    Completed {
        text: String,
        tokens_in: u32,
        tokens_out: u32,
        /// Stopped at `max_tokens` rather than at end of generation.
        #[serde(default)]
        truncated: bool,
        wall_s: f64,
    },
    Counted {
        tokens: u32,
    },
    EmbedLoaded {
        /// Vector length.
        dim: u32,
        load_s: f64,
    },
    Embedded {
        dim: u32,
        /// One unit-length vector per input text.
        vectors: Vec<Vec<f32>>,
    },
    /// Sent while a `complete` runs (after each prompt batch and every few
    /// output tokens), with the request's id, before its final reply. The
    /// parent treats it as a sign of life (protocol 2).
    Progress {
        tokens_in_done: u32,
        tokens_out: u32,
    },
    Health {
        loaded: bool,
    },
    Error {
        message: String,
    },
}

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_round_trip() {
        let reqs = [
            Request {
                id: 1,
                op: Op::Load {
                    model_path: "/m/x.gguf".into(),
                    n_ctx: 8192,
                    n_gpu_layers: 999,
                    seed: 7,
                    chat_format: "qwen3".into(),
                },
            },
            Request {
                id: 2,
                op: Op::Complete {
                    messages: vec![WireMessage {
                        role: "user".into(),
                        content: "xin chào".into(),
                    }],
                    schema: Some(json!({"type": "object"})),
                    max_tokens: 64,
                    temperature: 0.0,
                },
            },
            Request {
                id: 5,
                op: Op::Count {
                    text: "chào <|im_end|>".into(),
                },
            },
            Request {
                id: 6,
                op: Op::LoadEmbed {
                    model_path: "/m/e.gguf".into(),
                    max_tokens: 2048,
                    n_gpu_layers: 999,
                },
            },
            Request {
                id: 7,
                op: Op::Embed {
                    texts: vec!["họp quý bốn".into(), "q4 planning".into()],
                },
            },
            Request {
                id: 3,
                op: Op::Health,
            },
            Request {
                id: 4,
                op: Op::Shutdown,
            },
        ];
        for r in reqs {
            let line = serde_json::to_string(&r).unwrap();
            assert!(!line.contains('\n'));
            assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), r);
        }
    }

    #[test]
    fn wire_format_is_flat() {
        let line = serde_json::to_string(&Request {
            id: 9,
            op: Op::Health,
        })
        .unwrap();
        assert_eq!(line, r#"{"id":9,"op":"health"}"#);
        let r: Reply = serde_json::from_str(r#"{"id":9,"kind":"health","loaded":true}"#).unwrap();
        assert_eq!(r.body, Body::Health { loaded: true });
    }

    #[test]
    fn hello_round_trips() {
        let line = serde_json::to_string(&Hello::current()).unwrap();
        let h: Hello = serde_json::from_str(&line).unwrap();
        assert_eq!(h, Hello::current());
        assert_eq!(h.kind, "hello");
        assert_eq!(h.protocol, PROTOCOL);
    }

    #[test]
    fn replies_round_trip() {
        let replies = [
            Body::Loaded {
                n_ctx: 8192,
                load_s: 1.5,
            },
            Body::Completed {
                text: "Đã xong".into(),
                tokens_in: 10,
                tokens_out: 3,
                truncated: false,
                wall_s: 0.25,
            },
            Body::Counted { tokens: 12 },
            Body::EmbedLoaded {
                dim: 1024,
                load_s: 0.5,
            },
            Body::Embedded {
                dim: 2,
                vectors: vec![vec![0.6, 0.8], vec![-1.0, 0.0]],
            },
            Body::Health { loaded: false },
            Body::Error {
                message: "boom".into(),
            },
        ];
        for (i, body) in replies.into_iter().enumerate() {
            let r = Reply { id: i as u64, body };
            let line = serde_json::to_string(&r).unwrap();
            assert_eq!(serde_json::from_str::<Reply>(&line).unwrap(), r);
        }
    }
}
