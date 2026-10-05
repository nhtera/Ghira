// SPDX-License-Identifier: Apache-2.0
//! Speech engines behind one trait, so sessions and jobs run on NeMo-Speech.cpp
//! (feature `nemo`) or on a scripted fake in tests.
//!
//! Streams are owned (`'static`): they keep their model alive, so an engine
//! thread can hold them without borrowing.

use std::sync::Arc;

use ghi_speech::{AsrResult, AsrStream, DiarStream, SpeakerSegment, SpeechError, Word};

pub type Result<T> = std::result::Result<T, SpeechError>;
pub type BoxAsr = Box<dyn AsrStream + Send>;
pub type BoxDiar = Box<dyn DiarStream + Send>;

/// Opens streams on loaded models. One model per kind serves every track
/// (doc 06 §4.6); the library serializes compute internally.
pub trait SpeechEngines: Send + Sync {
    /// A streaming recognizer; `language` is a BCP-47 code or `None` (auto).
    fn asr(&self, language: Option<&str>) -> Result<BoxAsr>;
    fn diar(&self) -> Result<BoxDiar>;
    /// The ASR chunk this engine was loaded with (ms).
    fn chunk_ms(&self) -> u32;
    /// A chunk length (seconds) the final pass should use instead of its own:
    /// an engine that decodes at the end of a chunk (Whisper) wants short ones,
    /// so a recording can preempt the pass quickly.
    fn final_chunk_s(&self) -> Option<f64> {
        None
    }
    /// Names the engine and its models for the final pass's checkpoints: they
    /// are only reused by the same engine (another ASR, another model file).
    fn checkpoint_id(&self) -> String {
        format!("engine:{}", self.chunk_ms())
    }
}
/// `id:sha256` of the registry model a file is (callers verify the file
/// against the registry before loading); a file the registry doesn't know
/// falls back to `name:size`. Changes whenever the model is replaced.
#[cfg(feature = "nemo")]
fn model_tag(path: &std::path::Path) -> String {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    match ghi_models::registry().into_iter().find(|m| m.file == name) {
        Some(m) => format!("{}:{}", m.id, m.sha256),
        None => format!("{name}:{}", std::fs::metadata(path).map_or(0, |m| m.len())),
    }
}

#[cfg(feature = "nemo")]
pub use nemo::NemoEngines;

#[cfg(feature = "nemo")]
mod nemo {
    use std::path::Path;
    use std::sync::Arc;

    pub use ghi_speech::nemo::Device;
    use ghi_speech::nemo::{Asr, AsrConfig, AsrOptions, DiarConfig, Diarizer};

    use super::{BoxAsr, BoxDiar, Result, SpeechEngines};

    /// Nemotron 3.5 ASR + Nemotron 3 Diarization (`v3-streaming`).
    pub struct NemoEngines {
        asr: Arc<Asr>,
        diar: Arc<Diarizer>,
        chunk_ms: u32,
        tag: String,
    }

    impl NemoEngines {
        pub fn load(
            asr_model: &Path,
            diar_model: &Path,
            chunk_ms: u32,
            device: Device,
        ) -> Result<Self> {
            let asr = Asr::new(&AsrConfig {
                model: asr_model.to_path_buf(),
                device,
                chunk_ms: Some(chunk_ms),
                endpointing: true,
            })?;
            let diar = Diarizer::new(&DiarConfig {
                model: diar_model.to_path_buf(),
                device,
                preset: Some("v3-streaming".into()),
            })?;
            Ok(NemoEngines {
                asr: Arc::new(asr),
                diar: Arc::new(diar),
                chunk_ms,
                tag: format!(
                    "nemo:{}:{}:{chunk_ms}",
                    super::model_tag(asr_model),
                    super::model_tag(diar_model)
                ),
            })
        }
    }

    impl SpeechEngines for NemoEngines {
        fn asr(&self, language: Option<&str>) -> Result<BoxAsr> {
            let opts = AsrOptions {
                language: language.map(str::to_string),
            };
            Ok(Box::new(self.asr.stream_owned(&opts)?))
        }

        fn diar(&self) -> Result<BoxDiar> {
            Ok(Box::new(self.diar.stream_owned()?))
        }

        fn chunk_ms(&self) -> u32 {
            self.chunk_ms
        }

        fn checkpoint_id(&self) -> String {
            self.tag.clone()
        }
    }
}

#[cfg(all(feature = "nemo", feature = "whisper"))]
pub use whisper_final::WhisperFinalEngines;

/// The final pass with Whisper reading and NeMo (Sortformer) still diarizing.
/// Only the NeMo diarizer is loaded: with Whisper on, the Nemotron ASR stays
/// out of memory. Not for live capture (Whisper does not stream).
#[cfg(all(feature = "nemo", feature = "whisper"))]
mod whisper_final {
    use std::path::Path;
    use std::sync::Arc;

    use ghi_speech::nemo::{Device, DiarConfig, Diarizer};
    use ghi_speech::whisper::{Whisper, WhisperConfig, WhisperOptions};

    use super::{BoxAsr, BoxDiar, Result, SpeechEngines};

    pub struct WhisperFinalEngines {
        asr: Arc<Whisper>,
        diar: Arc<Diarizer>,
        chunk_ms: u32,
        tag: String,
    }

    impl WhisperFinalEngines {
        pub fn load(
            whisper_model: &Path,
            vad_model: &Path,
            diar_model: &Path,
            chunk_ms: u32,
            device: Device,
        ) -> Result<Self> {
            let asr = Whisper::new(&WhisperConfig {
                model: whisper_model.to_path_buf(),
                vad_model: vad_model.to_path_buf(),
                gpu: device == Device::Gpu,
                threads: 0,
            })?;
            let diar = Diarizer::new(&DiarConfig {
                model: diar_model.to_path_buf(),
                device,
                preset: Some("v3-streaming".into()),
            })?;
            Ok(Self {
                asr: Arc::new(asr),
                diar: Arc::new(diar),
                chunk_ms,
                tag: format!(
                    "whisper:{}:{}:{}",
                    super::model_tag(whisper_model),
                    super::model_tag(vad_model),
                    super::model_tag(diar_model)
                ),
            })
        }
    }

    impl SpeechEngines for WhisperFinalEngines {
        fn asr(&self, language: Option<&str>) -> Result<BoxAsr> {
            let opts = WhisperOptions {
                language: language.map(str::to_string),
            };
            Ok(Box::new(self.asr.stream_owned(&opts)))
        }

        fn diar(&self) -> Result<BoxDiar> {
            Ok(Box::new(self.diar.stream_owned()?))
        }

        fn chunk_ms(&self) -> u32 {
            self.chunk_ms
        }

        fn final_chunk_s(&self) -> Option<f64> {
            Some(120.0)
        }

        fn checkpoint_id(&self) -> String {
            self.tag.clone()
        }
    }
}

/// A scripted engine for tests: utterances and speaker turns at fixed times
/// (seconds of stream audio), produced once enough audio has been pushed.
#[derive(Debug, Clone, Default)]
pub struct Script {
    /// (start, end, text): one final per utterance, words spread evenly; a
    /// partial with the first word when the utterance starts.
    pub utterances: Vec<(f64, f64, String)>,
    /// Diarization turns.
    pub turns: Vec<SpeakerSegment>,
}

/// Fake engines over a [`Script`] per track kind (the same script for every
/// stream opened, so a reset stream "hears" the script from its own start).
pub struct FakeEngines {
    pub script: Script,
    pub chunk_ms: u32,
}

impl FakeEngines {
    pub fn new(script: Script) -> Arc<FakeEngines> {
        Arc::new(FakeEngines {
            script,
            chunk_ms: 560,
        })
    }
}

struct FakeAsr {
    script: Script,
    pushed: f64,
    next: usize,
    partial_sent: bool,
    finished: bool,
    out: Vec<AsrResult>,
}

impl AsrStream for FakeAsr {
    fn push(&mut self, pcm: &[f32], sample_rate: u32) -> Result<()> {
        self.pushed += pcm.len() as f64 / f64::from(sample_rate);
        self.produce();
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        self.finished = true;
        self.produce();
        Ok(())
    }

    fn next_result(&mut self) -> Result<Option<AsrResult>> {
        Ok((!self.out.is_empty()).then(|| self.out.remove(0)))
    }
}

impl FakeAsr {
    fn produce(&mut self) {
        while let Some((start, end, text)) = self.script.utterances.get(self.next).cloned() {
            if self.pushed < start {
                break;
            }
            let words: Vec<&str> = text.split_whitespace().collect();
            if !self.partial_sent && !words.is_empty() {
                self.out.push(AsrResult {
                    is_final: false,
                    text: words[0].to_string(),
                    words: Vec::new(),
                    languages: Vec::new(),
                    audio_processed: self.pushed,
                });
                self.partial_sent = true;
            }
            // A final comes once the utterance is over (or at finish).
            if self.pushed < end + 0.1 && !self.finished {
                break;
            }
            let step = (end - start) / words.len().max(1) as f64;
            let words = words
                .iter()
                .enumerate()
                .map(|(i, w)| Word {
                    text: (*w).to_string(),
                    start: start + i as f64 * step,
                    end: start + (i + 1) as f64 * step,
                    confidence: 0.9,
                    speaker: None,
                })
                .collect();
            self.out.push(AsrResult {
                is_final: true,
                text,
                words,
                languages: Vec::new(),
                audio_processed: self.pushed,
            });
            self.next += 1;
            self.partial_sent = false;
        }
    }
}

struct FakeDiar {
    turns: Vec<SpeakerSegment>,
    pushed: f64,
}

impl DiarStream for FakeDiar {
    fn push(&mut self, pcm: &[f32], sample_rate: u32) -> Result<()> {
        self.pushed += pcm.len() as f64 / f64::from(sample_rate);
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        self.pushed = f64::INFINITY;
        Ok(())
    }

    fn segments(&self) -> Result<Vec<SpeakerSegment>> {
        Ok(self
            .turns
            .iter()
            .filter(|t| t.start < self.pushed)
            .map(|t| SpeakerSegment {
                end: t.end.min(self.pushed),
                ..*t
            })
            .collect())
    }
}

impl SpeechEngines for FakeEngines {
    fn asr(&self, _language: Option<&str>) -> Result<BoxAsr> {
        Ok(Box::new(FakeAsr {
            script: self.script.clone(),
            pushed: 0.0,
            next: 0,
            partial_sent: false,
            finished: false,
            out: Vec::new(),
        }))
    }

    fn diar(&self) -> Result<BoxDiar> {
        Ok(Box::new(FakeDiar {
            turns: self.script.turns.clone(),
            pushed: 0.0,
        }))
    }

    fn chunk_ms(&self) -> u32 {
        self.chunk_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_asr_emits_partial_then_final_with_words() {
        let e = FakeEngines::new(Script {
            utterances: vec![(1.0, 2.0, "xin chào mọi người".into())],
            turns: vec![SpeakerSegment {
                start: 0.5,
                end: 2.5,
                speaker: 1,
            }],
        });
        let mut asr = e.asr(None).unwrap();
        let mut diar = e.diar().unwrap();
        let pcm = vec![0.0f32; 16_000];
        asr.push(&pcm, 16_000).unwrap();
        diar.push(&pcm, 16_000).unwrap();
        let r = asr.next_result().unwrap().unwrap();
        assert!(!r.is_final && r.text == "xin");
        assert!(asr.next_result().unwrap().is_none());
        asr.push(&pcm, 16_000).unwrap();
        asr.push(&pcm[..1_600], 16_000).unwrap();
        let r = asr.next_result().unwrap().unwrap();
        assert!(r.is_final);
        assert_eq!(r.words.len(), 4);
        assert_eq!((r.words[0].start, r.words[3].end), (1.0, 2.0));
        assert_eq!(diar.segments().unwrap()[0].end, 1.0, "only what was pushed");
    }
}
