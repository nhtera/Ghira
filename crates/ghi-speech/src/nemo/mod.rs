// SPDX-License-Identifier: Apache-2.0
//! NeMo-Speech.cpp backend: Nemotron 3.5 ASR and Nemotron 3 / Sortformer
//! diarization over the C ABI in [`sys`].
//!
//! Model handles are `Send + Sync` (the library serializes compute
//! internally); streams are `Send` only, one thread at a time.

pub mod sys;

use std::ffi::{CStr, CString, c_char};
use std::path::{Path, PathBuf};
use std::ptr::{self, NonNull};

use crate::{AsrResult, AsrStream, DiarStream, Result, SpeakerSegment, SpeechError, Word};

/// Library version string.
pub fn engine_version() -> String {
    // SAFETY: returns a static NUL-terminated string (or null).
    unsafe { string(sys::nemo_speech_asr_version()) }
}

/// Where to run inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Device {
    Cpu,
    /// GPU 0: Metal on Apple Silicon, Vulkan/CUDA where built.
    #[default]
    Gpu,
}

impl Device {
    fn index(self) -> i32 {
        match self {
            Device::Cpu => -1,
            Device::Gpu => 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AsrConfig {
    pub model: PathBuf,
    pub device: Device,
    /// Streaming chunk in ms: 80, 160, 320, 560 or 1120 (Nemotron 3.5). `None` = model default.
    pub chunk_ms: Option<u32>,
    /// Emit a final per utterance on trailing silence, instead of one final at the end.
    pub endpointing: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AsrOptions {
    /// BCP-47 code (`vi-VN`, `en-US`) or `None` for automatic detection.
    /// Streams always produce partial results (the C ABI has no switch for it).
    pub language: Option<String>,
}

/// A loaded ASR model.
pub struct Asr {
    ptr: NonNull<sys::Recognizer>,
}

// SAFETY: upstream src/asr/recognizer.h: "Recognizer is thread-safe; each
// RecognitionStream is driven by one caller thread". Streams are `Send` only.
unsafe impl Send for Asr {}
unsafe impl Sync for Asr {}

impl Asr {
    pub fn new(cfg: &AsrConfig) -> Result<Self> {
        let model = path_cstring(&cfg.model)?;
        let backend = sys::BackendConfig {
            size: size_of::<sys::BackendConfig>(),
            gpu: cfg.device.index(),
        };
        let model_cfg = sys::ModelConfig {
            size: size_of::<sys::ModelConfig>(),
            path: model.as_ptr(),
            name: ptr::null(),
        };
        let streaming = sys::StreamingConfig {
            size: size_of::<sys::StreamingConfig>(),
            // CTC-only fields (unused by the RNNT models); the library's documented defaults.
            chunk_size: 0.16,
            ctc_left_padding: 1.92,
            ctc_right_padding: 1.92,
            rnnt_right_context: match cfg.chunk_ms {
                Some(ms) => right_context_frames(ms)?,
                None => -1,
            },
        };
        let endpointing = sys::EndpointingConfig {
            size: size_of::<sys::EndpointingConfig>(),
            enable: cfg.endpointing,
            vad_based: false,
            stop_history_eou_ms: 800,
        };
        let config = sys::RecognizerConfig {
            size: size_of::<sys::RecognizerConfig>(),
            backend: &backend,
            model: &model_cfg,
            streaming: &streaming,
            decoder: ptr::null(),
            vad: ptr::null(),
            endpointing: &endpointing,
            postproc: ptr::null(),
            diar: ptr::null(),
            batching: ptr::null(),
        };
        let mut out = ptr::null_mut();
        // SAFETY: all pointers in `config` outlive the call; `out` is written on OK.
        check("nemo_speech_asr_create", unsafe {
            sys::nemo_speech_asr_create(&config, &mut out)
        })?;
        Ok(Self {
            ptr: non_null(out, "nemo_speech_asr_create")?,
        })
    }

    /// Decodes a whole recording in one call (offline). Unlike streaming, the
    /// result reports detected languages.
    pub fn recognize(&self, pcm: &[f32], sample_rate: u32, opts: &AsrOptions) -> Result<AsrResult> {
        let (raw, _lang) = raw_options(opts)?;
        let mut out = ptr::null_mut();
        // SAFETY: `raw`, its language string and `pcm` live across the call;
        // `out` is a result we destroy after copying.
        check("nemo_speech_asr_recognize_f32", unsafe {
            sys::nemo_speech_asr_recognize_f32(
                self.ptr.as_ptr(),
                &raw,
                pcm.as_ptr(),
                pcm.len(),
                rate(sample_rate)?,
                &mut out,
            )
        })?;
        let out = non_null(out, "nemo_speech_asr_recognize_f32")?;
        // SAFETY: live result, destroyed right after copying.
        let result = unsafe { read_result(out.as_ptr()) };
        unsafe { sys::nemo_speech_asr_result_destroy(out.as_ptr()) };
        Ok(result)
    }

    /// Starts a streaming recognition.
    pub fn stream(&self, opts: &AsrOptions) -> Result<NemoAsrStream<'_>> {
        let (raw, _lang) = raw_options(opts)?;
        let mut out = ptr::null_mut();
        // SAFETY: `raw` and the language string it points to live across the call.
        check("nemo_speech_asr_streaming_recognize", unsafe {
            sys::nemo_speech_asr_streaming_recognize(self.ptr.as_ptr(), &raw, &mut out)
        })?;
        Ok(NemoAsrStream {
            ptr: non_null(out, "nemo_speech_asr_streaming_recognize")?,
            _asr: std::marker::PhantomData,
        })
    }
}

impl Drop for Asr {
    fn drop(&mut self) {
        // SAFETY: created by nemo_speech_asr_create; streams borrow `self`, so none outlive it.
        unsafe { sys::nemo_speech_asr_destroy(self.ptr.as_ptr()) }
    }
}

/// A streaming recognition borrowed from an [`Asr`].
pub struct NemoAsrStream<'a> {
    ptr: NonNull<sys::Stream>,
    _asr: std::marker::PhantomData<&'a Asr>,
}

// SAFETY: a stream may move between threads but is used by one at a time (`&mut self`).
unsafe impl Send for NemoAsrStream<'_> {}

impl AsrStream for NemoAsrStream<'_> {
    fn push(&mut self, pcm: &[f32], sample_rate: u32) -> Result<()> {
        // SAFETY: `pcm` is valid for `pcm.len()` floats during the call.
        check("nemo_speech_asr_stream_push_f32", unsafe {
            sys::nemo_speech_asr_stream_push_f32(
                self.ptr.as_ptr(),
                pcm.as_ptr(),
                pcm.len(),
                rate(sample_rate)?,
            )
        })
    }

    fn finish(&mut self) -> Result<()> {
        // SAFETY: valid stream handle.
        check("nemo_speech_asr_stream_finish", unsafe {
            sys::nemo_speech_asr_stream_finish(self.ptr.as_ptr())
        })
    }

    fn next_result(&mut self) -> Result<Option<AsrResult>> {
        let mut out = ptr::null_mut();
        // SAFETY: valid stream handle; `out` is null or a result we must destroy.
        check("nemo_speech_asr_stream_next", unsafe {
            sys::nemo_speech_asr_stream_next(self.ptr.as_ptr(), &mut out)
        })?;
        if out.is_null() {
            return Ok(None);
        }
        // SAFETY: `out` is a live result; it is destroyed right after copying.
        let result = unsafe { read_result(out) };
        unsafe { sys::nemo_speech_asr_result_destroy(out) };
        Ok(Some(result))
    }
}

impl Drop for NemoAsrStream<'_> {
    fn drop(&mut self) {
        // SAFETY: valid stream handle, closed once.
        unsafe { sys::nemo_speech_asr_stream_close(self.ptr.as_ptr()) }
    }
}

#[derive(Debug, Clone)]
pub struct DiarConfig {
    pub model: PathBuf,
    pub device: Device,
    /// `v3-streaming` / `v3-offline` for Nemotron 3 Diarization; `None` = model default.
    pub preset: Option<String>,
}

/// A loaded diarization model.
pub struct Diarizer {
    ptr: NonNull<sys::DiarModel>,
}

// SAFETY: independent streams may run on different threads; compute
// serializes internally (diar.h).
unsafe impl Send for Diarizer {}
unsafe impl Sync for Diarizer {}

impl Diarizer {
    pub fn new(cfg: &DiarConfig) -> Result<Self> {
        let model = path_cstring(&cfg.model)?;
        let preset = cfg.preset.as_deref().map(cstring).transpose()?;
        let config = sys::DiarModelConfig {
            size: size_of::<sys::DiarModelConfig>(),
            model_path: model.as_ptr(),
            gpu: cfg.device.index(),
            preset: preset.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
            // <= 0 keeps the preset (left context: < 0).
            chunk_frames: 0,
            right_context_frames: 0,
            left_context_frames: -1,
            fifo_frames: 0,
            spkcache_frames: 0,
            update_period_frames: 0,
        };
        let mut out = ptr::null_mut();
        // SAFETY: strings outlive the call; `out` is written on OK.
        check("nemo_speech_diar_create", unsafe {
            sys::nemo_speech_diar_create(&config, &mut out)
        })?;
        Ok(Self {
            ptr: non_null(out, "nemo_speech_diar_create")?,
        })
    }

    /// Maximum number of speakers the model tracks (Nemotron 3: 8, Sortformer v2: 4).
    pub fn num_speakers(&self) -> u32 {
        // SAFETY: valid model handle.
        unsafe { sys::nemo_speech_diar_num_speakers(self.ptr.as_ptr()) }.max(0) as u32
    }

    pub fn stream(&self) -> Result<NemoDiarStream<'_>> {
        let mut out = ptr::null_mut();
        // SAFETY: valid model handle; the stream borrows `self`.
        check("nemo_speech_diar_stream_open", unsafe {
            sys::nemo_speech_diar_stream_open(self.ptr.as_ptr(), &mut out)
        })?;
        Ok(NemoDiarStream {
            ptr: non_null(out, "nemo_speech_diar_stream_open")?,
            _model: std::marker::PhantomData,
        })
    }
}

impl Drop for Diarizer {
    fn drop(&mut self) {
        // SAFETY: created by nemo_speech_diar_create; streams borrow `self`.
        unsafe { sys::nemo_speech_diar_destroy(self.ptr.as_ptr()) }
    }
}

pub struct NemoDiarStream<'a> {
    ptr: NonNull<sys::DiarStream>,
    _model: std::marker::PhantomData<&'a Diarizer>,
}

// SAFETY: single-threaded by contract, enforced by `&mut self`; may move threads.
unsafe impl Send for NemoDiarStream<'_> {}

impl DiarStream for NemoDiarStream<'_> {
    fn push(&mut self, pcm: &[f32], sample_rate: u32) -> Result<()> {
        // SAFETY: `pcm` is valid for `pcm.len()` floats during the call.
        check("nemo_speech_diar_stream_push_f32", unsafe {
            sys::nemo_speech_diar_stream_push_f32(
                self.ptr.as_ptr(),
                pcm.as_ptr(),
                pcm.len(),
                rate(sample_rate)?,
            )
        })
    }

    fn finish(&mut self) -> Result<()> {
        // SAFETY: valid stream handle.
        check("nemo_speech_diar_stream_finish", unsafe {
            sys::nemo_speech_diar_stream_finish(self.ptr.as_ptr())
        })
    }

    fn segments(&self) -> Result<Vec<SpeakerSegment>> {
        let mut count = 0usize;
        // SAFETY: two-call pattern: null buffer returns the count.
        check("nemo_speech_diar_segments", unsafe {
            sys::nemo_speech_diar_segments(
                self.ptr.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                0,
                &mut count,
            )
        })?;
        let mut buf = vec![sys::Segment::default(); count];
        // SAFETY: `buf` holds `count` segments; the library writes at most that many.
        check("nemo_speech_diar_segments", unsafe {
            sys::nemo_speech_diar_segments(
                self.ptr.as_ptr(),
                ptr::null(),
                buf.as_mut_ptr(),
                buf.len(),
                &mut count,
            )
        })?;
        buf.truncate(count);
        Ok(buf
            .into_iter()
            .map(|s| SpeakerSegment {
                start: s.start_time,
                end: s.end_time,
                speaker: s.speaker.max(0) as u32,
            })
            .collect())
    }
}

impl Drop for NemoDiarStream<'_> {
    fn drop(&mut self) {
        // SAFETY: valid stream handle, closed once.
        unsafe { sys::nemo_speech_diar_stream_close(self.ptr.as_ptr()) }
    }
}

/// Nemotron 3.5 chunk sizes are (R + 1) × 80 ms for cache-aware right context R.
fn right_context_frames(chunk_ms: u32) -> Result<i32> {
    match chunk_ms {
        80 | 160 | 320 | 560 | 1120 => Ok((chunk_ms / 80) as i32 - 1),
        other => Err(SpeechError {
            op: "asr config",
            message: format!("chunk_ms must be 80, 160, 320, 560 or 1120, not {other}"),
        }),
    }
}

/// Options struct plus the owned language string it points into.
fn raw_options(opts: &AsrOptions) -> Result<(sys::RecognitionOptions, Option<CString>)> {
    // SAFETY: returns a value, no pointers owned by the library.
    let mut raw = unsafe { sys::nemo_speech_asr_recognition_options_default() };
    let lang = opts.language.as_deref().map(cstring).transpose()?;
    raw.language_code = lang.as_ref().map_or(ptr::null(), |l| l.as_ptr());
    raw.interim_results = true;
    raw.enable_word_time_offsets = true;
    raw.enable_automatic_punctuation = true;
    Ok((raw, lang))
}

/// Copies a result out of library memory.
///
/// # Safety
/// `r` must be a live result handle.
unsafe fn read_result(r: *const sys::AsrResult) -> AsrResult {
    unsafe {
        let has_alt = sys::nemo_speech_asr_result_alternative_count(r) > 0;
        let words = if has_alt {
            (0..sys::nemo_speech_asr_result_word_count(r, 0))
                .map(|i| {
                    let tag = sys::nemo_speech_asr_result_word_speaker_tag(r, 0, i);
                    Word {
                        text: string(sys::nemo_speech_asr_result_word_text(r, 0, i)),
                        start: f64::from(sys::nemo_speech_asr_result_word_start_time(r, 0, i))
                            / 1000.0,
                        end: f64::from(sys::nemo_speech_asr_result_word_end_time(r, 0, i)) / 1000.0,
                        confidence: sys::nemo_speech_asr_result_word_confidence(r, 0, i),
                        speaker: (tag > 0).then_some(tag as u32),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        let languages = if has_alt {
            (0..sys::nemo_speech_asr_result_language_count(r, 0))
                .map(|i| string(sys::nemo_speech_asr_result_language_code(r, 0, i)))
                .collect()
        } else {
            Vec::new()
        };
        AsrResult {
            is_final: sys::nemo_speech_asr_result_is_final(r),
            text: if has_alt {
                string(sys::nemo_speech_asr_result_transcript(r, 0))
            } else {
                String::new()
            },
            words,
            languages,
            audio_processed: f64::from(sys::nemo_speech_asr_result_audio_processed(r)),
        }
    }
}

/// Copies a library-owned C string (null = empty).
///
/// # Safety
/// `p` must be null or a valid NUL-terminated string.
unsafe fn string(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        // SAFETY: caller guarantees a valid C string.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}

fn check(op: &'static str, status: sys::Status) -> Result<()> {
    if status == sys::OK {
        return Ok(());
    }
    // SAFETY: thread-local message for the call that just failed on this thread.
    let detail = unsafe { string(sys::nemo_speech_asr_last_error()) };
    Err(SpeechError {
        op,
        message: format!("status {status}: {detail}"),
    })
}

fn non_null<T>(p: *mut T, op: &'static str) -> Result<NonNull<T>> {
    NonNull::new(p).ok_or(SpeechError {
        op,
        message: "returned OK with a null handle".into(),
    })
}

fn rate(sample_rate: u32) -> Result<i32> {
    i32::try_from(sample_rate).map_err(|_| SpeechError {
        op: "push",
        message: format!("sample rate {sample_rate} out of range"),
    })
}

fn cstring(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| SpeechError {
        op: "config",
        message: "string contains a NUL byte".into(),
    })
}

fn path_cstring(p: &Path) -> Result<CString> {
    if !p.is_file() {
        return Err(SpeechError {
            op: "config",
            message: format!("model file not found: {}", p.display()),
        });
    }
    cstring(&p.to_string_lossy())
}

/// A streaming recognition that keeps its model alive (an `Arc`), so it can
/// be stored and moved without borrowing (the core's engine threads, phase 8).
pub struct OwnedAsrStream {
    // Field order: the stream is dropped (closed) before the model.
    stream: NemoAsrStream<'static>,
    _asr: std::sync::Arc<Asr>,
}

impl Asr {
    /// Like [`Asr::stream`], holding a reference to the model.
    pub fn stream_owned(self: &std::sync::Arc<Self>, opts: &AsrOptions) -> Result<OwnedAsrStream> {
        let stream = self.stream(opts)?;
        // SAFETY: the stream only needs the model to outlive it; the `Arc`
        // stored next to it keeps the model alive, and it is dropped after
        // the stream (field order).
        let stream: NemoAsrStream<'static> = unsafe { std::mem::transmute(stream) };
        Ok(OwnedAsrStream {
            stream,
            _asr: self.clone(),
        })
    }
}

impl AsrStream for OwnedAsrStream {
    fn push(&mut self, pcm: &[f32], sample_rate: u32) -> Result<()> {
        self.stream.push(pcm, sample_rate)
    }
    fn finish(&mut self) -> Result<()> {
        self.stream.finish()
    }
    fn next_result(&mut self) -> Result<Option<AsrResult>> {
        self.stream.next_result()
    }
}

/// A diarization stream that keeps its model alive (see [`OwnedAsrStream`]).
pub struct OwnedDiarStream {
    stream: NemoDiarStream<'static>,
    _model: std::sync::Arc<Diarizer>,
}

impl Diarizer {
    /// Like [`Diarizer::stream`], holding a reference to the model.
    pub fn stream_owned(self: &std::sync::Arc<Self>) -> Result<OwnedDiarStream> {
        let stream = self.stream()?;
        // SAFETY: as in `Asr::stream_owned`.
        let stream: NemoDiarStream<'static> = unsafe { std::mem::transmute(stream) };
        Ok(OwnedDiarStream {
            stream,
            _model: self.clone(),
        })
    }
}

impl DiarStream for OwnedDiarStream {
    fn push(&mut self, pcm: &[f32], sample_rate: u32) -> Result<()> {
        self.stream.push(pcm, sample_rate)
    }
    fn finish(&mut self) -> Result<()> {
        self.stream.finish()
    }
    fn segments(&self) -> Result<Vec<SpeakerSegment>> {
        self.stream.segments()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_sizes_map_to_right_context() {
        assert_eq!(right_context_frames(80).unwrap(), 0);
        assert_eq!(right_context_frames(560).unwrap(), 6);
        assert_eq!(right_context_frames(1120).unwrap(), 13);
        assert!(right_context_frames(500).is_err());
    }

    #[test]
    fn missing_model_is_an_error() {
        let err = Asr::new(&AsrConfig {
            model: "no-such-model.gguf".into(),
            device: Device::Cpu,
            chunk_ms: None,
            endpointing: false,
        })
        .err()
        .unwrap();
        assert!(err.message.contains("not found"));
    }

    #[test]
    fn engine_reports_a_version() {
        assert!(!engine_version().is_empty());
    }
}
