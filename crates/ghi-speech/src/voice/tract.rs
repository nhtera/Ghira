// SPDX-License-Identifier: Apache-2.0
//! CAM++ on tract. tract cannot plan a symbolic frame count for this graph and
//! is only exact for whole 200-frame multiples (its last pooling segment is
//! wrong otherwise), so there is one plan per bucket.

use std::io::Cursor;
use std::path::Path;

use tract_onnx::prelude::*;

use super::fbank::{self, Fbank, NUM_MEL};
use super::{BUCKETS, EMBEDDING_DIM, MIN_RMS, VoiceEmbedder};
use crate::{Result, SpeechError};

type Plan = std::sync::Arc<TypedRunnableModel>;

fn err(op: &'static str, e: impl std::fmt::Display) -> SpeechError {
    SpeechError {
        op,
        message: e.to_string(),
    }
}

pub struct TractVoice {
    fbank: Fbank,
    /// The model file, read once; each bucket's plan is built from it on first use.
    model: Vec<u8>,
    plans: [Option<Plan>; BUCKETS.len()],
}

impl TractVoice {
    /// Reads and checks the model file. Plans (one per bucket) are built on
    /// first use, a fraction of a second each, so memory follows what is used.
    /// The caller verifies the file hash first (`ghi_models::verify_for_load`).
    pub fn open(path: &Path) -> Result<Self> {
        let model = std::fs::read(path).map_err(|e| err("voice_open", e))?;
        tract_onnx::onnx()
            .model_for_read(&mut Cursor::new(&model))
            .map_err(|e| err("voice_open", format!("{e:#}")))?;
        Ok(Self {
            fbank: Fbank::new(),
            model,
            plans: Default::default(),
        })
    }

    fn plan(&mut self, bucket: usize) -> Result<&Plan> {
        if self.plans[bucket].is_none() {
            let t = BUCKETS[bucket];
            let plan = tract_onnx::onnx()
                .model_for_read(&mut Cursor::new(&self.model))
                .and_then(|m| m.with_input_fact(0, f32::fact([1, t, NUM_MEL]).into()))
                .and_then(|m| m.into_optimized())
                .and_then(|m| m.into_runnable())
                .map_err(|e| err("voice_open", format!("{e:#}")))?;
            self.plans[bucket] = Some(plan);
        }
        Ok(self.plans[bucket].as_ref().expect("just built"))
    }
}

impl VoiceEmbedder for TractVoice {
    fn embed(&mut self, pcm16k: &[f32]) -> Result<Option<Vec<f32>>> {
        let frames = fbank::num_frames(pcm16k.len());
        let Some(bucket) = BUCKETS.iter().rposition(|&t| t <= frames) else {
            return Ok(None);
        };
        let t = BUCKETS[bucket];
        let window = &pcm16k[..fbank::samples_for_frames(t)];
        if window.iter().any(|x| !x.is_finite()) {
            return Err(err("voice_embed", "non-finite sample in input"));
        }
        let energy = window.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>();
        if (energy / window.len() as f64).sqrt() < MIN_RMS {
            return Ok(None);
        }
        let feats = self.fbank.features(window);
        let input =
            Tensor::from_shape(&[1, t, NUM_MEL], &feats).map_err(|e| err("voice_embed", e))?;
        let out = self
            .plan(bucket)?
            .run(tvec!(input.into()))
            .map_err(|e| err("voice_embed", format!("{e:#}")))?;
        let v = out[0]
            .to_plain_array_view::<f32>()
            .map_err(|e| err("voice_embed", e))?
            .iter()
            .copied()
            .collect::<Vec<_>>();
        if v.len() != EMBEDDING_DIM {
            return Err(err(
                "voice_embed",
                format!("expected {EMBEDDING_DIM} values, got {}", v.len()),
            ));
        }
        if v.iter().any(|x| !x.is_finite()) {
            return Err(err("voice_embed", "non-finite value in embedding"));
        }
        Ok(Some(v))
    }
}
