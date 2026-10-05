// SPDX-License-Identifier: Apache-2.0
//! Whisper large-v3-turbo final-pass ASR over a pinned, statically linked
//! whisper.cpp (see `tools/scripts/build-whisper.sh`, `shim.c`).
//!
//! Not a streaming engine: [`WhisperStream`] buffers what it is pushed and, on
//! `finish`, finds speech with Silero VAD, packs the regions into windows of
//! at most 29 s, decodes each with DTW word timestamps and drops what looks
//! invented (see [`plan`]). The model handle is `Send + Sync`; decoding is
//! serialized by a lock, as whisper.cpp does not allow two calls on a context.

pub mod plan;
pub mod sys;

use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_void};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::{AsrResult, AsrStream, Result, SpeechError};
use plan::{MIN_GROUP_S, RawSeg, RawTok, Region, Window};

/// The audio rate Whisper and the VAD take.
pub const SAMPLE_RATE: u32 = 16_000;

/// Library version string.
pub fn engine_version() -> String {
    // SAFETY: returns a static NUL-terminated string.
    unsafe { string(sys::ghi_wh_version()) }
}

#[derive(Debug, Clone)]
pub struct WhisperConfig {
    /// `ggml-large-v3-turbo-q5_0.bin` (the DTW alignment heads are the turbo's).
    pub model: PathBuf,
    /// `ggml-silero-v6.2.0.bin`.
    pub vad_model: PathBuf,
    /// Metal on Apple Silicon.
    pub gpu: bool,
    /// CPU threads for the parts that run there; 0 = 4.
    pub threads: u32,
}

#[derive(Debug, Clone, Default)]
pub struct WhisperOptions {
    /// BCP-47 code (`vi-VN`, `en-US`) or `None` to detect per window, between
    /// English and Vietnamese.
    pub language: Option<String>,
}

/// A loaded Whisper model and VAD.
pub struct Whisper {
    ptr: Mutex<NonNull<sys::Ctx>>,
}

// SAFETY: every use of the context goes through the mutex.
unsafe impl Send for Whisper {}
unsafe impl Sync for Whisper {}

impl Drop for Whisper {
    fn drop(&mut self) {
        let p = *self.ptr.get_mut().unwrap_or_else(|e| e.into_inner());
        // SAFETY: created by ghi_wh_new, freed once.
        unsafe { sys::ghi_wh_free(p.as_ptr()) };
    }
}

impl Whisper {
    pub fn new(cfg: &WhisperConfig) -> Result<Self> {
        let model = path_cstring(&cfg.model)?;
        let vad = path_cstring(&cfg.vad_model)?;
        // SAFETY: both strings outlive the call.
        let ptr = unsafe {
            sys::ghi_wh_new(
                model.as_ptr(),
                vad.as_ptr(),
                i32::from(cfg.gpu),
                cfg.threads as i32,
            )
        };
        let ptr = NonNull::new(ptr).ok_or_else(|| SpeechError {
            op: "ghi_wh_new",
            message: format!(
                "could not load {} and {}",
                cfg.model.display(),
                cfg.vad_model.display()
            ),
        })?;
        Ok(Self {
            ptr: Mutex::new(ptr),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, NonNull<sys::Ctx>> {
        self.ptr.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Speech regions of 16 kHz mono audio.
    pub fn speech_regions(&self, pcm: &[f32]) -> Result<Vec<Region>> {
        let guard = self.lock();
        let w = guard.as_ptr();
        let n = i32::try_from(pcm.len()).map_err(|_| too_long("ghi_wh_vad"))?;
        // Silero's usual settings; padding 0.3 s each side (the context
        // Whisper wants), and a cap a little under the window so a long
        // monologue is cut at a pause rather than mid-word.
        // SAFETY: `pcm` lives across the call; the segments stay in the handle.
        let rc = unsafe { sys::ghi_wh_vad(w, pcm.as_ptr(), n, 0.5, 250, 300, 25.0, 300) };
        if rc != 0 {
            return Err(SpeechError {
                op: "ghi_wh_vad",
                message: "voice activity detection failed".into(),
            });
        }
        // SAFETY: indexes are below the count the handle just reported.
        Ok(unsafe {
            (0..sys::ghi_wh_vad_count(w))
                .map(|i| Region {
                    start: sys::ghi_wh_vad_t0(w, i),
                    end: sys::ghi_wh_vad_t1(w, i),
                })
                .collect()
        })
    }

    /// Decodes one window of at most 30 s: its segments and the language used,
    /// or `None` when `abort` stopped it.
    fn decode(
        &self,
        pcm: &[f32],
        language: Option<&str>,
        abort: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<(Vec<RawSeg>, String)>> {
        let lang = language.map(whisper_code).map(cstring).transpose()?;
        let guard = self.lock();
        let w = guard.as_ptr();
        let n = i32::try_from(pcm.len()).map_err(|_| too_long("ghi_wh_decode"))?;
        let state = AbortState {
            f: abort,
            hit: AtomicBool::new(false),
        };
        // SAFETY: `pcm`, the language string and `state` live across the call.
        let rc = unsafe {
            sys::ghi_wh_decode(
                w,
                pcm.as_ptr(),
                n,
                lang.as_ref().map_or(std::ptr::null(), |l| l.as_ptr()),
                Some(abort_tramp),
                &state as *const AbortState as *mut c_void,
            )
        };
        if state.hit.load(Ordering::SeqCst) {
            return Ok(None);
        }
        if rc != 0 {
            return Err(SpeechError {
                op: "ghi_wh_decode",
                message: format!("whisper_full failed ({rc})"),
            });
        }
        // SAFETY: the handle's results are valid until the next call, which
        // the lock keeps out; every string is copied here.
        unsafe {
            let language = string(sys::ghi_wh_lang(w));
            let segs = (0..sys::ghi_wh_seg_count(w))
                .map(|i| RawSeg {
                    t0: sys::ghi_wh_seg_t0(w, i),
                    t1: sys::ghi_wh_seg_t1(w, i),
                    no_speech: sys::ghi_wh_seg_no_speech(w, i),
                    text: string(sys::ghi_wh_seg_text(w, i)),
                    toks: (0..sys::ghi_wh_tok_count(w, i))
                        .filter(|&t| sys::ghi_wh_tok_is_text(w, i, t) != 0)
                        .map(|t| {
                            let dtw = sys::ghi_wh_tok_dtw(w, i, t);
                            RawTok {
                                bytes: bytes(sys::ghi_wh_tok_text(w, i, t)),
                                p: sys::ghi_wh_tok_p(w, i, t),
                                t: (dtw >= 0.0).then_some(dtw),
                            }
                        })
                        .collect(),
                })
                .collect();
            Ok(Some((segs, language)))
        }
    }

    /// Transcribes a whole recording: finals, in time order, with word times
    /// from the start of `pcm` (16 kHz mono). No speech gives no results.
    pub fn recognize(&self, pcm: &[f32], opts: &WhisperOptions) -> Result<Vec<AsrResult>> {
        Ok(self
            .recognize_abortable(pcm, opts, &|| false)?
            .unwrap_or_default())
    }

    /// [`Whisper::recognize`], stopping early (`None`) once `abort` says so:
    /// checked between windows and inside each decode.
    pub fn recognize_abortable(
        &self,
        pcm: &[f32],
        opts: &WhisperOptions,
        abort: &(dyn Fn() -> bool + Sync),
    ) -> Result<Option<Vec<AsrResult>>> {
        let rate = f64::from(SAMPLE_RATE);
        let regions = self.speech_regions(pcm)?;
        let mut out = Vec::new();
        for window in plan::plan_windows(&regions) {
            if abort() {
                return Ok(None);
            }
            let audio = window_audio(pcm, &window);
            let win_len = audio.len() as f64 / rate;
            let Some((segs, lang)) = self.decode(&audio, opts.language.as_deref(), abort)? else {
                return Ok(None);
            };
            for seg in segs {
                // Words and the guard work in window time; the times are mapped
                // back to the recording afterwards.
                let mut words = plan::words(&seg, 0.0, win_len);
                if !plan::keep(&seg, &words, seg.t0, seg.t1.min(win_len), &window.speech) {
                    continue;
                }
                for w in &mut words {
                    w.start = window.to_source(w.start, true);
                    w.end = window.to_source(w.end, false).max(w.start);
                }
                let end = words.last().map_or(0.0, |w| w.end);
                out.push(AsrResult {
                    is_final: true,
                    text: seg.text.trim().to_string(),
                    words,
                    languages: vec![bcp47(&lang)],
                    audio_processed: end,
                });
            }
        }
        Ok(Some(out))
    }

    /// A stream that keeps the model alive.
    pub fn stream_owned(self: &Arc<Self>, opts: &WhisperOptions) -> WhisperStream {
        WhisperStream {
            model: self.clone(),
            opts: opts.clone(),
            pcm: Vec::new(),
            out: VecDeque::new(),
            done: false,
        }
    }
}

/// Buffers 16 kHz audio and transcribes it at `finish` (see the module docs).
pub struct WhisperStream {
    model: Arc<Whisper>,
    opts: WhisperOptions,
    pcm: Vec<f32>,
    out: VecDeque<AsrResult>,
    done: bool,
}

impl AsrStream for WhisperStream {
    fn push(&mut self, pcm: &[f32], sample_rate: u32) -> Result<()> {
        if sample_rate != SAMPLE_RATE {
            return Err(SpeechError {
                op: "whisper_push",
                message: format!("expects {SAMPLE_RATE} Hz audio, got {sample_rate}"),
            });
        }
        self.pcm.extend_from_slice(pcm);
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        self.finish_abortable(&|| false).map(|_| ())
    }

    fn finish_abortable(&mut self, abort: &(dyn Fn() -> bool + Sync)) -> Result<bool> {
        if !self.done {
            // The audio stays until the decode completes, so an aborted
            // stream can finish again later.
            match self
                .model
                .recognize_abortable(&self.pcm, &self.opts, abort)?
            {
                Some(results) => {
                    self.done = true;
                    self.pcm = Vec::new();
                    self.out.extend(results);
                }
                None => return Ok(false),
            }
        }
        Ok(true)
    }

    fn next_result(&mut self) -> Result<Option<AsrResult>> {
        Ok(self.out.pop_front())
    }
}

/// The window's audio: its pieces of `pcm` with [`plan::SEP_S`] of silence
/// between, padded to [`MIN_GROUP_S`] (Whisper does poorly on very short audio).
fn window_audio(pcm: &[f32], window: &Window) -> Vec<f32> {
    let rate = f64::from(SAMPLE_RATE);
    let at = |t: f64| ((t * rate) as usize).min(pcm.len());
    let mut audio: Vec<f32> = Vec::new();
    for p in &window.pieces {
        // Where the piece belongs in the window, silence in between.
        audio.resize((p.at * rate) as usize, 0.0);
        audio.extend_from_slice(&pcm[at(p.src.start)..at(p.src.end).max(at(p.src.start))]);
    }
    audio.resize(audio.len().max((MIN_GROUP_S * rate) as usize), 0.0);
    audio
}

struct AbortState<'a> {
    f: &'a (dyn Fn() -> bool + Sync),
    hit: AtomicBool,
}

/// whisper.cpp's abort callback (may run on a worker thread).
///
/// # Safety
/// `ud` is the `AbortState` that `decode` keeps alive across the call.
unsafe extern "C" fn abort_tramp(ud: *mut c_void) -> bool {
    let state = unsafe { &*(ud as *const AbortState) };
    if (state.f)() {
        state.hit.store(true, Ordering::SeqCst);
    }
    state.hit.load(Ordering::SeqCst)
}

/// `vi-VN` / `en-US` to Whisper's `vi` / `en`.
fn whisper_code(tag: &str) -> String {
    tag.split(['-', '_']).next().unwrap_or(tag).to_lowercase()
}

/// Whisper's `vi` / `en` to the engines' BCP-47 `vi-VN` / `en-US`.
fn bcp47(code: &str) -> String {
    match code {
        "vi" => "vi-VN".into(),
        "en" => "en-US".into(),
        other => other.to_string(),
    }
}

fn path_cstring(p: &Path) -> Result<CString> {
    cstring(p.to_string_lossy().into_owned())
}

fn cstring(s: String) -> Result<CString> {
    CString::new(s).map_err(|_| SpeechError {
        op: "whisper",
        message: "string contains a NUL byte".into(),
    })
}

fn too_long(op: &'static str) -> SpeechError {
    SpeechError {
        op,
        message: "audio is too long for one call".into(),
    }
}

/// # Safety
/// `p` is null or a NUL-terminated string.
unsafe fn bytes(p: *const c_char) -> Vec<u8> {
    if p.is_null() {
        Vec::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()
    }
}

/// # Safety
/// As [`bytes`].
unsafe fn string(p: *const c_char) -> String {
    String::from_utf8_lossy(&unsafe { bytes(p) }).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_tags_map_both_ways() {
        assert_eq!(whisper_code("vi-VN"), "vi");
        assert_eq!(whisper_code("en_US"), "en");
        assert_eq!(whisper_code("FR"), "fr");
        assert_eq!(bcp47("vi"), "vi-VN");
        assert_eq!(bcp47("en"), "en-US");
        assert_eq!(bcp47("fr"), "fr");
    }

    #[test]
    fn links_the_pinned_library() {
        assert!(engine_version().starts_with("1.9"), "{}", engine_version());
    }
}
