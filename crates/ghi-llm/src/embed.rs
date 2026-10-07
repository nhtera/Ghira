// SPDX-License-Identifier: Apache-2.0
//! Text embeddings for semantic search: the [`Embedder`] interface, the local
//! model over its own worker process ([`LocalEmbedder`]) and a deterministic
//! fake for tests ([`FakeEmbedder`]).
//!
//! Vectors are unit length, so a dot product is the cosine similarity. The
//! embedder is a separate worker from the notes LLM; the two are never needed
//! at once.

use std::path::Path;
use std::time::Duration;

use crate::sidecar::{Body, Op, Sidecar};
use crate::{LlmError, Result};

/// What a text is for: a search query gets the model's instruction prefix,
/// a document (a transcript chunk) is embedded as is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Query,
    Document,
}

pub trait Embedder {
    /// One unit-length vector of [`Embedder::dim`] numbers per text, in order.
    fn embed(&mut self, texts: &[String], kind: Kind) -> Result<Vec<Vec<f32>>>;
    fn dim(&self) -> usize;
    /// Registry id of the model; stored with every vector, so a model change
    /// is a re-index, never a mix.
    fn model_id(&self) -> &str;
}

/// Cosine similarity (a dot product for unit vectors; a zero vector gives 0).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// The task line of Qwen3-Embedding's query format.
const QUERY_TASK: &str =
    "Given a question about a meeting, retrieve the transcript passages that answer it";

/// `Instruct: <task>\nQuery:<q>`, the format the model was trained with
/// (documents carry no prefix).
pub fn query_text(q: &str) -> String {
    format!("Instruct: {QUERY_TASK}\nQuery:{q}")
}

/// Texts per worker request.
const BATCH: usize = 16;
/// Longest text embedded, in tokens (a transcript chunk is far below it).
const MAX_TOKENS: u32 = 2048;
/// Offload every layer (Metal on Apple silicon).
const ALL_LAYERS: u32 = 999;
const LOAD_TIMEOUT: Duration = Duration::from_secs(120);
const BATCH_TIMEOUT: Duration = Duration::from_secs(120);

/// An embedding model loaded in its own worker process. Dropping it kills
/// the worker, which frees the model memory.
pub struct LocalEmbedder {
    sidecar: Sidecar,
    model_id: String,
    dim: usize,
}

impl LocalEmbedder {
    pub fn open(model_path: &Path, model_id: &str) -> Result<LocalEmbedder> {
        let path = model_path
            .to_str()
            .ok_or_else(|| LlmError::Invalid("model path is not valid UTF-8".into()))?;
        let mut sidecar = crate::sidecar::start()?;
        let reply = sidecar.request(
            Op::LoadEmbed {
                model_path: path.to_string(),
                max_tokens: MAX_TOKENS,
                n_gpu_layers: ALL_LAYERS,
            },
            LOAD_TIMEOUT,
        )?;
        match reply.body {
            Body::EmbedLoaded { dim, .. } => Ok(LocalEmbedder {
                sidecar,
                model_id: model_id.to_string(),
                dim: dim as usize,
            }),
            Body::Error { message } => Err(LlmError::Worker(message)),
            _ => Err(LlmError::Worker("unexpected reply to load_embed".into())),
        }
    }

    /// Opens a registry model from `dir`, checking its pinned SHA-256 first.
    pub fn open_registry_in(dir: &Path, id: &str) -> Result<LocalEmbedder> {
        let model = ghi_models::find(id)
            .ok_or_else(|| LlmError::Invalid(format!("unknown model id {id}")))?;
        if model.role != "embed" {
            return Err(LlmError::Invalid(format!("model {id} is not an embedder")));
        }
        let path = ghi_models::path_in(dir, &model);
        ghi_models::verify_for_load(&path, &model)
            .map_err(|e| LlmError::Worker(format!("model {id}: {e}")))?;
        log::info!("embed model load id={id}");
        LocalEmbedder::open(&path, &model.id)
    }

    /// Kill the worker (kill = unload).
    pub fn unload(mut self) {
        self.sidecar.kill();
    }
}

impl Embedder for LocalEmbedder {
    fn embed(&mut self, texts: &[String], kind: Kind) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for batch in texts.chunks(BATCH) {
            let texts = match kind {
                Kind::Query => batch.iter().map(|q| query_text(q)).collect(),
                Kind::Document => batch.to_vec(),
            };
            let reply = self.sidecar.request(Op::Embed { texts }, BATCH_TIMEOUT)?;
            match reply.body {
                Body::Embedded { dim, vectors } => {
                    if dim as usize != self.dim
                        || vectors.len() != batch.len()
                        || vectors.iter().any(|v| v.len() != self.dim)
                    {
                        return Err(LlmError::Worker("embedding shape mismatch".into()));
                    }
                    out.extend(vectors);
                }
                Body::Error { message } => return Err(LlmError::Worker(message)),
                _ => return Err(LlmError::Worker("unexpected reply to embed".into())),
            }
        }
        Ok(out)
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

/// A deterministic stand-in for tests: each lowercase word hashes into one of
/// [`FakeEmbedder::DIM`] buckets, so texts sharing words are similar. Counts
/// the texts embedded and can be told to fail.
#[derive(Debug, Default)]
pub struct FakeEmbedder {
    /// Texts embedded so far (queries included).
    pub embedded: usize,
    /// The next `embed` calls fail with a worker error while this is true.
    pub fail: bool,
}

impl FakeEmbedder {
    pub const DIM: usize = 64;
    pub const MODEL: &'static str = "fake-embed";

    pub fn new() -> FakeEmbedder {
        FakeEmbedder::default()
    }

    fn vector(text: &str) -> Vec<f32> {
        let mut v = vec![0f32; Self::DIM];
        for word in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            // FNV-1a: stable across runs and platforms.
            let h = word
                .to_lowercase()
                .bytes()
                .fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
                });
            v[(h % Self::DIM as u64) as usize] += 1.0;
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            v.iter_mut().for_each(|x| *x /= norm);
        }
        v
    }
}

impl Embedder for FakeEmbedder {
    fn embed(&mut self, texts: &[String], _kind: Kind) -> Result<Vec<Vec<f32>>> {
        if self.fail {
            return Err(LlmError::Worker("fake embedder failure".into()));
        }
        self.embedded += texts.len();
        Ok(texts.iter().map(|t| Self::vector(t)).collect())
    }

    fn dim(&self) -> usize {
        Self::DIM
    }

    fn model_id(&self) -> &str {
        Self::MODEL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_vectors_are_unit_and_share_words() {
        let mut e = FakeEmbedder::new();
        let v = e
            .embed(
                &[
                    "chốt ngân sách quý bốn".into(),
                    "ngân sách quý bốn đã chốt".into(),
                    "mua laptop mới".into(),
                ],
                Kind::Document,
            )
            .unwrap();
        for x in &v {
            assert!((cosine(x, x) - 1.0).abs() < 1e-5);
        }
        assert!(cosine(&v[0], &v[1]) > cosine(&v[0], &v[2]));
        assert_eq!(e.embedded, 3);
        assert_eq!((e.dim(), e.model_id()), (64, "fake-embed"));
    }

    #[test]
    fn query_format_carries_the_instruction() {
        let q = query_text("who owns the budget?");
        assert!(q.starts_with("Instruct: "));
        assert!(q.ends_with("\nQuery:who owns the budget?"));
    }

    #[test]
    fn cosine_of_zero_is_zero() {
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
    }
}
