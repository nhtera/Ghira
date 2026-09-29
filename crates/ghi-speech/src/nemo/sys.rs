// SPDX-License-Identifier: Apache-2.0
//! Hand-written bindings for NeMo-Speech.cpp's stable C ABI
//! (`include/nemo_speech/{asr,diar}.h`, pinned in `third_party/`). The ABI is
//! append-only with size-prefixed structs, so only the fields Ghira uses need
//! to be right; `tests::layout_matches_c` checks every struct against the C
//! compiler. Upstream changes stay inside this file (RT-15).

#![allow(non_camel_case_types)]

use std::ffi::c_char;

pub type Status = i32;
pub const OK: Status = 0;

#[repr(C)]
pub struct Recognizer {
    _private: [u8; 0],
}
#[repr(C)]
pub struct Stream {
    _private: [u8; 0],
}
#[repr(C)]
pub struct AsrResult {
    _private: [u8; 0],
}
#[repr(C)]
pub struct DiarModel {
    _private: [u8; 0],
}
#[repr(C)]
pub struct DiarStream {
    _private: [u8; 0],
}

#[repr(C)]
pub struct BackendConfig {
    pub size: usize,
    /// -1 = CPU.
    pub gpu: i32,
}

#[repr(C)]
pub struct ModelConfig {
    pub size: usize,
    pub path: *const c_char,
    pub name: *const c_char,
}

#[repr(C)]
pub struct StreamingConfig {
    pub size: usize,
    pub chunk_size: f32,
    pub ctc_left_padding: f32,
    pub ctc_right_padding: f32,
    /// Cache-aware right context in 80 ms encoder frames; -1 = model default.
    pub rnnt_right_context: i32,
}

#[repr(C)]
pub struct DecoderConfig {
    pub size: usize,
    pub kind: i32,
    pub flashlight_lm: *const c_char,
    pub flashlight_lexicon: *const c_char,
    pub flashlight_tokenizer: *const c_char,
    pub beam_size: i32,
    pub beam_size_token: i32,
    pub beam_threshold: f64,
    pub lm_weight: f64,
    pub word_insertion_score: f64,
    pub max_boost: f64,
}

#[repr(C)]
pub struct VadConfig {
    pub size: usize,
    pub model_path: *const c_char,
    pub enable_masking: bool,
    pub onset: f32,
    pub offset: f32,
}

#[repr(C)]
pub struct EndpointingConfig {
    pub size: usize,
    pub enable: bool,
    pub vad_based: bool,
    pub stop_history_eou_ms: i32,
}

#[repr(C)]
pub struct PostprocConfig {
    pub size: usize,
    pub profanity_list_path: *const c_char,
    pub itn_model_dir: *const c_char,
    pub pnc_model_path: *const c_char,
}

#[repr(C)]
pub struct AsrDiarConfig {
    pub size: usize,
    pub model_path: *const c_char,
    pub chunk_frames: i32,
    pub right_context_frames: i32,
    pub left_context_frames: i32,
    pub fifo_frames: i32,
    pub spkcache_frames: i32,
    pub update_period_frames: i32,
}

#[repr(C)]
pub struct BatchingConfig {
    pub size: usize,
    pub enable: bool,
    pub max_batch_size: i32,
    pub max_queue_delay_us: i32,
    pub max_queue_depth: i32,
    pub ingress_cohort_delay_us: i32,
    pub state_arena_slots: i32,
}

/// Every sub-config is optional (null = library default).
#[repr(C)]
pub struct RecognizerConfig {
    pub size: usize,
    pub backend: *const BackendConfig,
    pub model: *const ModelConfig,
    pub streaming: *const StreamingConfig,
    pub decoder: *const DecoderConfig,
    pub vad: *const VadConfig,
    pub endpointing: *const EndpointingConfig,
    pub postproc: *const PostprocConfig,
    pub diar: *const AsrDiarConfig,
    pub batching: *const BatchingConfig,
}

#[repr(C)]
pub struct SpeechContext {
    pub size: usize,
    pub phrases: *const *const c_char,
    pub phrase_count: usize,
    pub boost: f32,
}

#[repr(C)]
pub struct RecognitionOptions {
    pub size: usize,
    pub request_id: *const c_char,
    /// Prompt selection; null or "" = auto.
    pub language_code: *const c_char,
    pub interim_results: bool,
    pub enable_word_time_offsets: bool,
    pub enable_automatic_punctuation: bool,
    pub verbatim_transcripts: bool,
    pub profanity_filter: bool,
    pub stop_history_eou_ms: i32,
    pub speech_contexts: *const SpeechContext,
    pub speech_context_count: usize,
    pub max_alternatives: i32,
    pub enable_speaker_diarization: bool,
    pub max_speaker_count: i32,
}

#[repr(C)]
pub struct DiarModelConfig {
    pub size: usize,
    pub model_path: *const c_char,
    pub gpu: i32,
    /// "v3-streaming" / "v3-offline" (Nemotron 3), "streaming" / "offline" (V2); null = model default.
    pub preset: *const c_char,
    pub chunk_frames: i32,
    pub right_context_frames: i32,
    pub left_context_frames: i32,
    pub fifo_frames: i32,
    pub spkcache_frames: i32,
    pub update_period_frames: i32,
}

#[repr(C)]
pub struct SegmentationConfig {
    pub size: usize,
    pub onset: f32,
    pub offset: f32,
    pub pad_onset_sec: f64,
    pub pad_offset_sec: f64,
    pub min_gap_sec: f64,
    pub min_duration_sec: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Segment {
    pub start_time: f64,
    pub end_time: f64,
    /// 1-based.
    pub speaker: i32,
}

unsafe extern "C" {
    pub fn nemo_speech_asr_recognition_options_default() -> RecognitionOptions;
    pub fn nemo_speech_asr_create(
        cfg: *const RecognizerConfig,
        out: *mut *mut Recognizer,
    ) -> Status;
    pub fn nemo_speech_asr_destroy(recognizer: *mut Recognizer);
    pub fn nemo_speech_asr_recognize_f32(
        recognizer: *mut Recognizer,
        options: *const RecognitionOptions,
        samples: *const f32,
        n_samples: usize,
        sample_rate: i32,
        out: *mut *mut AsrResult,
    ) -> Status;
    pub fn nemo_speech_asr_streaming_recognize(
        recognizer: *mut Recognizer,
        options: *const RecognitionOptions,
        out: *mut *mut Stream,
    ) -> Status;
    pub fn nemo_speech_asr_stream_push_f32(
        stream: *mut Stream,
        samples: *const f32,
        n_samples: usize,
        sample_rate: i32,
    ) -> Status;
    pub fn nemo_speech_asr_stream_finish(stream: *mut Stream) -> Status;
    pub fn nemo_speech_asr_stream_next(stream: *mut Stream, out: *mut *mut AsrResult) -> Status;
    pub fn nemo_speech_asr_stream_close(stream: *mut Stream);

    pub fn nemo_speech_asr_result_is_final(result: *const AsrResult) -> bool;
    pub fn nemo_speech_asr_result_audio_processed(result: *const AsrResult) -> f32;
    pub fn nemo_speech_asr_result_alternative_count(result: *const AsrResult) -> usize;
    pub fn nemo_speech_asr_result_transcript(result: *const AsrResult, alt: usize)
    -> *const c_char;
    pub fn nemo_speech_asr_result_word_count(result: *const AsrResult, alt: usize) -> usize;
    pub fn nemo_speech_asr_result_word_text(
        result: *const AsrResult,
        alt: usize,
        i: usize,
    ) -> *const c_char;
    pub fn nemo_speech_asr_result_word_start_time(
        result: *const AsrResult,
        alt: usize,
        i: usize,
    ) -> i32;
    pub fn nemo_speech_asr_result_word_end_time(
        result: *const AsrResult,
        alt: usize,
        i: usize,
    ) -> i32;
    pub fn nemo_speech_asr_result_word_confidence(
        result: *const AsrResult,
        alt: usize,
        i: usize,
    ) -> f32;
    pub fn nemo_speech_asr_result_word_speaker_tag(
        result: *const AsrResult,
        alt: usize,
        i: usize,
    ) -> i32;
    pub fn nemo_speech_asr_result_language_count(result: *const AsrResult, alt: usize) -> usize;
    pub fn nemo_speech_asr_result_language_code(
        result: *const AsrResult,
        alt: usize,
        i: usize,
    ) -> *const c_char;
    pub fn nemo_speech_asr_result_destroy(result: *mut AsrResult);

    pub fn nemo_speech_asr_last_error() -> *const c_char;
    pub fn nemo_speech_asr_version() -> *const c_char;

    pub fn nemo_speech_diar_create(cfg: *const DiarModelConfig, out: *mut *mut DiarModel)
    -> Status;
    pub fn nemo_speech_diar_destroy(model: *mut DiarModel);
    pub fn nemo_speech_diar_num_speakers(model: *const DiarModel) -> i32;
    pub fn nemo_speech_diar_seconds_per_frame(model: *const DiarModel) -> f64;
    pub fn nemo_speech_diar_stream_open(model: *mut DiarModel, out: *mut *mut DiarStream)
    -> Status;
    pub fn nemo_speech_diar_stream_push_f32(
        stream: *mut DiarStream,
        samples: *const f32,
        n_samples: usize,
        sample_rate: i32,
    ) -> Status;
    pub fn nemo_speech_diar_stream_finish(stream: *mut DiarStream) -> Status;
    pub fn nemo_speech_diar_stream_close(stream: *mut DiarStream);
    pub fn nemo_speech_diar_frame_count(stream: *const DiarStream) -> i64;
    pub fn nemo_speech_diar_segments(
        stream: *const DiarStream,
        cfg: *const SegmentationConfig,
        out: *mut Segment,
        capacity: usize,
        count: *mut usize,
    ) -> Status;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    unsafe extern "C" {
        fn ghi_nemo_layout(i: usize) -> usize;
    }

    #[test]
    fn layout_matches_c() {
        let rust = [
            size_of::<BackendConfig>(),
            offset_of!(BackendConfig, gpu),
            size_of::<ModelConfig>(),
            offset_of!(ModelConfig, name),
            size_of::<StreamingConfig>(),
            offset_of!(StreamingConfig, rnnt_right_context),
            size_of::<DecoderConfig>(),
            offset_of!(DecoderConfig, beam_size),
            offset_of!(DecoderConfig, max_boost),
            size_of::<VadConfig>(),
            offset_of!(VadConfig, offset),
            size_of::<EndpointingConfig>(),
            offset_of!(EndpointingConfig, stop_history_eou_ms),
            size_of::<PostprocConfig>(),
            offset_of!(PostprocConfig, pnc_model_path),
            size_of::<AsrDiarConfig>(),
            offset_of!(AsrDiarConfig, update_period_frames),
            size_of::<BatchingConfig>(),
            offset_of!(BatchingConfig, state_arena_slots),
            size_of::<RecognizerConfig>(),
            offset_of!(RecognizerConfig, batching),
            size_of::<SpeechContext>(),
            offset_of!(SpeechContext, boost),
            size_of::<RecognitionOptions>(),
            offset_of!(RecognitionOptions, profanity_filter),
            offset_of!(RecognitionOptions, stop_history_eou_ms),
            offset_of!(RecognitionOptions, speech_context_count),
            offset_of!(RecognitionOptions, max_alternatives),
            offset_of!(RecognitionOptions, enable_speaker_diarization),
            offset_of!(RecognitionOptions, max_speaker_count),
            size_of::<DiarModelConfig>(),
            offset_of!(DiarModelConfig, preset),
            offset_of!(DiarModelConfig, update_period_frames),
            size_of::<SegmentationConfig>(),
            offset_of!(SegmentationConfig, pad_onset_sec),
            offset_of!(SegmentationConfig, min_duration_sec),
            size_of::<Segment>(),
            offset_of!(Segment, speaker),
            // Mid-struct fields Ghira writes.
            offset_of!(RecognitionOptions, language_code),
            offset_of!(RecognitionOptions, interim_results),
            offset_of!(RecognitionOptions, enable_word_time_offsets),
            offset_of!(RecognitionOptions, enable_automatic_punctuation),
            offset_of!(EndpointingConfig, enable),
            offset_of!(EndpointingConfig, vad_based),
            offset_of!(ModelConfig, path),
            offset_of!(RecognizerConfig, model),
            offset_of!(RecognizerConfig, endpointing),
            offset_of!(DiarModelConfig, model_path),
            offset_of!(DiarModelConfig, gpu),
        ];
        for (i, value) in rust.iter().enumerate() {
            assert_eq!(*value, unsafe { ghi_nemo_layout(i) }, "layout entry {i}");
        }
        assert_eq!(
            unsafe { ghi_nemo_layout(rust.len()) },
            usize::MAX,
            "entry count"
        );
    }

    #[test]
    fn default_options_have_our_size() {
        let opts = unsafe { nemo_speech_asr_recognition_options_default() };
        assert_eq!(opts.size, size_of::<RecognitionOptions>());
    }
}
