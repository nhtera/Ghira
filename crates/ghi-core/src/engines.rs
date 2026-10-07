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
    /// Whether the live engine should reopen its ASR stream after this final
    /// line (a fresh stream for the next utterance).
    fn reset_after(&self, _line: &str) -> bool {
        false
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
            Self::load_tuned(asr_model, diar_model, chunk_ms, None, device)
        }

        /// [`NemoEngines::load`] with the trailing silence (ms) that ends an
        /// utterance; `None` = the library default (the live bench tunes it).
        pub fn load_tuned(
            asr_model: &Path,
            diar_model: &Path,
            chunk_ms: u32,
            eou_ms: Option<u32>,
            device: Device,
        ) -> Result<Self> {
            let asr = Asr::new(&AsrConfig {
                model: asr_model.to_path_buf(),
                device,
                chunk_ms: Some(chunk_ms),
                endpointing: true,
                eou_ms,
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
                // Pauses decide where lines end: another value is another pass.
                tag: format!(
                    "nemo:{}:{}:{chunk_ms}{}",
                    super::model_tag(asr_model),
                    super::model_tag(diar_model),
                    eou_ms
                        .filter(|&e| e != ghi_speech::nemo::EOU_MS)
                        .map_or(String::new(), |e| format!(":eou{e}"))
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

        /// After a Vietnamese line (Vietnamese is written with diacritics): a
        /// fresh Nemotron stream reads Vietnamese and code-switched speech
        /// better (live bench, 2026-10-06: ViMedCSS 35% -> 19% WER, VietMed
        /// 22% -> 18%, FLEURS vi 16% -> 15%), while English reads as well or
        /// better with its context kept (AMI, Earnings-21 flat).
        fn reset_after(&self, line: &str) -> bool {
            ghi_text::has_diacritics(line)
        }
    }
}

#[cfg(all(feature = "nemo", feature = "whisper"))]
pub use whisper_final::WhisperFinalEngines;

/// The final pass with Whisper reading and NeMo (Sortformer) still diarizing.
/// Where Whisper's guard drops text as invented (a video sign-off over real
/// speech), Nemotron reads that stretch instead, so the transcript has no
/// gaps; the Nemotron ASR is loaded only when the first gap needs it. Not for
/// live capture (Whisper does not stream).
#[cfg(all(feature = "nemo", feature = "whisper"))]
mod whisper_final {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use ghi_speech::nemo::{Asr, AsrConfig, AsrOptions, Device, DiarConfig, Diarizer};
    use ghi_speech::whisper::{Fallback, Whisper, WhisperConfig, WhisperOptions};
    use ghi_speech::{AsrResult, AsrStream};

    use super::{BoxAsr, BoxDiar, Result, SpeechEngines};

    pub struct WhisperFinalEngines {
        asr: Arc<Whisper>,
        diar: Arc<Diarizer>,
        gap_asr: Arc<GapAsr>,
        chunk_ms: u32,
        tag: String,
    }

    /// The Nemotron ASR that fills Whisper's gaps, loaded on first use.
    struct GapAsr {
        model: PathBuf,
        chunk_ms: u32,
        device: Device,
        loaded: Mutex<Option<Arc<Asr>>>,
    }

    impl GapAsr {
        fn get(&self) -> Result<Arc<Asr>> {
            let mut slot = self.loaded.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(asr) = &*slot {
                return Ok(asr.clone());
            }
            log::info!("whisper gap fill loads asr=nemotron");
            let asr = Arc::new(Asr::new(&AsrConfig {
                model: self.model.clone(),
                device: self.device,
                chunk_ms: Some(self.chunk_ms),
                endpointing: true,
                eou_ms: None,
            })?);
            *slot = Some(asr.clone());
            Ok(asr)
        }

        /// Reads one stretch on its own: its final lines, times from its start.
        fn read(&self, pcm: &[f32], language: Option<&str>) -> Result<Vec<AsrResult>> {
            let mut stream = self.get()?.stream_owned(&AsrOptions {
                language: language.map(str::to_string),
            })?;
            stream.push(pcm, ghi_speech::whisper::SAMPLE_RATE)?;
            stream.finish()?;
            let mut out = Vec::new();
            while let Some(r) = stream.next_result()? {
                if r.is_final && !r.text.trim().is_empty() {
                    out.push(r);
                }
            }
            Ok(out)
        }
    }

    impl WhisperFinalEngines {
        /// `gap_asr_model`: the Nemotron ASR that reads what Whisper's guard
        /// drops (see the type docs).
        pub fn load(
            whisper_model: &Path,
            vad_model: &Path,
            diar_model: &Path,
            gap_asr_model: &Path,
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
                gap_asr: Arc::new(GapAsr {
                    model: gap_asr_model.to_path_buf(),
                    chunk_ms,
                    device,
                    loaded: Mutex::new(None),
                }),
                chunk_ms,
                tag: format!(
                    "whisper:{}:{}:{}:gaps:{}",
                    super::model_tag(whisper_model),
                    super::model_tag(vad_model),
                    super::model_tag(diar_model),
                    super::model_tag(gap_asr_model)
                ),
            })
        }
    }

    impl SpeechEngines for WhisperFinalEngines {
        fn asr(&self, language: Option<&str>) -> Result<BoxAsr> {
            let opts = WhisperOptions {
                language: language.map(str::to_string),
            };
            let gaps = self.gap_asr.clone();
            let lang = opts.language.clone();
            let fallback: Fallback = Box::new(move |pcm: &[f32]| {
                // A gap left empty beats a failed pass.
                Ok(gaps.read(pcm, lang.as_deref()).unwrap_or_else(|e| {
                    log::warn!("whisper gap fill failed: {e}");
                    Vec::new()
                }))
            });
            Ok(Box::new(
                self.asr.stream_owned(&opts).with_fallback(fallback),
            ))
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

/// One talker reading `words` back to back, one word per `step` seconds from
/// `start`, with a second of silence (an utterance ends) after the word counts
/// in `pauses`. Unlike [`Script`], times are on the meeting clock.
#[derive(Debug, Clone, Default)]
pub struct Talk {
    pub words: Vec<String>,
    pub start: f64,
    pub step: f64,
    pub pauses: Vec<usize>,
    /// Speaker turns (meeting seconds); none: one speaker throughout.
    pub turns: Vec<SpeakerSegment>,
    /// A word shows in the partial this long after it ends (a real streaming
    /// recognizer decodes a chunk behind).
    pub show_lag: f64,
    /// The diarizer reports turns this far behind the audio.
    pub diar_lag: f64,
}

impl Talk {
    fn word_end(&self, i: usize) -> f64 {
        let pauses = self.pauses.iter().filter(|&&p| p <= i).count();
        self.start + (i + 1) as f64 * self.step + pauses as f64
    }

    /// Words that began before `t` (meeting seconds).
    fn begun(&self, t: f64) -> usize {
        (0..self.words.len())
            .take_while(|&i| self.word_end(i) - self.step < t)
            .count()
    }
}

/// Fake engines over a [`Talk`] for one track, where streams share the meeting
/// clock (counted by the diarizer, which hears every block once): a reopened
/// ASR stream hears only what follows, as a real one does. A line with
/// diacritics asks for a fresh stream, as NeMo's does.
pub struct TalkEngines {
    talk: Talk,
    clock: Arc<std::sync::Mutex<(f64, u32)>>,
}

impl TalkEngines {
    pub fn new(talk: Talk) -> Arc<TalkEngines> {
        Arc::new(TalkEngines {
            talk,
            clock: Arc::default(),
        })
    }

    /// ASR streams opened so far.
    pub fn opened(&self) -> u32 {
        self.clock.lock().unwrap_or_else(|e| e.into_inner()).1
    }
}

struct TalkAsr {
    talk: Talk,
    clock: Arc<std::sync::Mutex<(f64, u32)>>,
    /// Meeting time when the stream opened.
    origin: f64,
    /// Next word not yet in a final.
    next: usize,
    partial: String,
    out: Vec<AsrResult>,
    /// Finished, and its last final (finishing again repeats it, as NeMo does).
    finished: Option<Option<AsrResult>>,
}

impl TalkAsr {
    fn result(&self, from: usize, to: usize, is_final: bool) -> AsrResult {
        let t = &self.talk;
        AsrResult {
            is_final,
            text: t.words[from..to].join(" "),
            words: (from..to)
                .map(|i| Word {
                    text: t.words[i].clone(),
                    start: t.word_end(i) - t.step - self.origin,
                    end: t.word_end(i) - self.origin,
                    confidence: 0.9,
                    speaker: None,
                })
                .collect(),
            languages: Vec::new(),
            audio_processed: 0.0,
        }
    }

    /// Finishing flushes what was buffered: a word cut by the flush is this
    /// stream's (the next one hears only what follows).
    fn produce(&mut self, finishing: bool) {
        let now = self.clock.lock().unwrap_or_else(|e| e.into_inner()).0;
        let t = &self.talk;
        let heard = if finishing {
            t.begun(now)
        } else {
            (0..t.words.len())
                .take_while(|&i| t.word_end(i) + t.show_lag <= now)
                .count()
        };
        self.next = self.next.max(t.begun(self.origin));
        let end = match t.pauses.iter().find(|&&p| p > self.next && p <= heard) {
            Some(&p) => Some(p),
            None if finishing && heard > self.next => Some(heard),
            None => None,
        };
        if let Some(end) = end {
            self.out.push(self.result(self.next, end, true));
            self.next = end;
            self.partial.clear();
        } else if heard > self.next {
            let p = t.words[self.next..heard].join(" ");
            if p != self.partial {
                self.partial = p;
                self.out.push(self.result(self.next, heard, false));
            }
        }
    }
}

impl AsrStream for TalkAsr {
    fn push(&mut self, _: &[f32], _: u32) -> Result<()> {
        self.produce(false);
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        if let Some(last) = &self.finished {
            self.out.extend(last.clone());
            return Ok(());
        }
        self.produce(true);
        self.finished = Some(self.out.iter().rfind(|r| r.is_final).cloned());
        Ok(())
    }
    fn next_result(&mut self) -> Result<Option<AsrResult>> {
        Ok((!self.out.is_empty()).then(|| self.out.remove(0)))
    }
}

/// The talk's turns as heard so far (one speaker without turns); it moves
/// the meeting clock.
struct TalkDiar {
    turns: Vec<SpeakerSegment>,
    lag: f64,
    clock: Arc<std::sync::Mutex<(f64, u32)>>,
}

impl DiarStream for TalkDiar {
    fn push(&mut self, pcm: &[f32], sample_rate: u32) -> Result<()> {
        self.clock.lock().unwrap_or_else(|e| e.into_inner()).0 +=
            pcm.len() as f64 / f64::from(sample_rate);
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
    fn segments(&self) -> Result<Vec<SpeakerSegment>> {
        let now = self.clock.lock().unwrap_or_else(|e| e.into_inner()).0 - self.lag;
        if self.turns.is_empty() {
            return Ok(vec![SpeakerSegment {
                start: 0.0,
                end: now,
                speaker: 1,
            }]);
        }
        Ok(self
            .turns
            .iter()
            .filter(|t| t.start < now)
            .map(|t| SpeakerSegment {
                end: t.end.min(now),
                ..*t
            })
            .collect())
    }
}

impl SpeechEngines for TalkEngines {
    fn asr(&self, _language: Option<&str>) -> Result<BoxAsr> {
        let mut c = self.clock.lock().unwrap_or_else(|e| e.into_inner());
        c.1 += 1;
        Ok(Box::new(TalkAsr {
            talk: self.talk.clone(),
            clock: self.clock.clone(),
            origin: c.0,
            next: 0,
            partial: String::new(),
            out: Vec::new(),
            finished: None,
        }))
    }

    fn diar(&self) -> Result<BoxDiar> {
        Ok(Box::new(TalkDiar {
            turns: self.talk.turns.clone(),
            lag: self.talk.diar_lag,
            clock: self.clock.clone(),
        }))
    }

    fn chunk_ms(&self) -> u32 {
        560
    }

    fn reset_after(&self, line: &str) -> bool {
        ghi_text::has_diacritics(line)
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
