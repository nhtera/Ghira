// SPDX-License-Identifier: Apache-2.0
// Sizes and field offsets of the NeMo-Speech.cpp ABI structs, as the C compiler
// sees them. `sys.rs` tests compare them with the hand-written Rust structs.
#include <stddef.h>
#include "nemo_speech/asr.h"
#include "nemo_speech/diar.h"

#define S(t) sizeof(t)
#define O(t, f) offsetof(t, f)

static const size_t layout[] = {
    S(nemo_speech_asr_backend_config), O(nemo_speech_asr_backend_config, gpu),
    S(nemo_speech_asr_model_config), O(nemo_speech_asr_model_config, name),
    S(nemo_speech_asr_streaming_config), O(nemo_speech_asr_streaming_config, rnnt_right_context),
    S(nemo_speech_asr_decoder_config), O(nemo_speech_asr_decoder_config, beam_size),
    O(nemo_speech_asr_decoder_config, max_boost),
    S(nemo_speech_asr_vad_config), O(nemo_speech_asr_vad_config, offset),
    S(nemo_speech_asr_endpointing_config), O(nemo_speech_asr_endpointing_config, stop_history_eou_ms),
    S(nemo_speech_asr_postproc_config), O(nemo_speech_asr_postproc_config, pnc_model_path),
    S(nemo_speech_asr_diar_config), O(nemo_speech_asr_diar_config, update_period_frames),
    S(nemo_speech_asr_batching_config), O(nemo_speech_asr_batching_config, state_arena_slots),
    S(nemo_speech_asr_recognizer_config), O(nemo_speech_asr_recognizer_config, batching),
    S(nemo_speech_asr_speech_context), O(nemo_speech_asr_speech_context, boost),
    S(nemo_speech_asr_recognition_options),
    O(nemo_speech_asr_recognition_options, profanity_filter),
    O(nemo_speech_asr_recognition_options, stop_history_eou_ms),
    O(nemo_speech_asr_recognition_options, speech_context_count),
    O(nemo_speech_asr_recognition_options, max_alternatives),
    O(nemo_speech_asr_recognition_options, enable_speaker_diarization),
    O(nemo_speech_asr_recognition_options, max_speaker_count),
    S(nemo_speech_diar_model_config), O(nemo_speech_diar_model_config, preset),
    O(nemo_speech_diar_model_config, update_period_frames),
    S(nemo_speech_diar_segmentation_config), O(nemo_speech_diar_segmentation_config, pad_onset_sec),
    O(nemo_speech_diar_segmentation_config, min_duration_sec),
    S(nemo_speech_diar_segment), O(nemo_speech_diar_segment, speaker),
    // Mid-struct fields Ghira writes.
    O(nemo_speech_asr_recognition_options, language_code),
    O(nemo_speech_asr_recognition_options, interim_results),
    O(nemo_speech_asr_recognition_options, enable_word_time_offsets),
    O(nemo_speech_asr_recognition_options, enable_automatic_punctuation),
    O(nemo_speech_asr_endpointing_config, enable), O(nemo_speech_asr_endpointing_config, vad_based),
    O(nemo_speech_asr_model_config, path),
    O(nemo_speech_asr_recognizer_config, model), O(nemo_speech_asr_recognizer_config, endpointing),
    O(nemo_speech_diar_model_config, model_path), O(nemo_speech_diar_model_config, gpu),
};

size_t ghi_nemo_layout(size_t i) {
    return i < sizeof(layout) / sizeof(layout[0]) ? layout[i] : (size_t)-1;
}
