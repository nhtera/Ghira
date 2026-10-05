// SPDX-License-Identifier: Apache-2.0
// A flat C face over the pinned whisper.cpp C API, compiled against its own
// `whisper.h`. whisper_full_params and whisper_context_params are large and
// change between releases; keeping them on this side means Rust only sees plain
// ints, floats and pointers, and a pin bump that breaks a field fails to
// compile here instead of corrupting a hand-written struct.
//
// Decoded results stay inside the handle (`ghi_wh_seg_*`, `ghi_wh_tok_*`) until
// the next decode or VAD call on it; the Rust side copies them out at once.
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "whisper.h"

struct ghi_wh {
    struct whisper_context * ctx;
    struct whisper_vad_context * vad;
    struct whisper_vad_segments * vad_segments;
    char lang[8];
    // The last language a decode settled on, "vi" or "en" (the fallback when a
    // re-check cannot be made).
    char last_allowed[3];
    int n_threads;
};

static void quiet_log(enum ggml_log_level level, const char * text, void * user) {
    (void) user;
    // Errors only: whisper.cpp is chatty at INFO (model dumps, timings).
    // (An aborted decode reports "failed to encode/decode": not an error.)
    if (level == GGML_LOG_LEVEL_ERROR && !strstr(text, "failed to encode") &&
        !strstr(text, "failed to decode")) {
        fputs(text, stderr);
    }
}

const char * ghi_wh_version(void) { return whisper_version(); }

struct ghi_wh * ghi_wh_new(const char * model, const char * vad_model, int use_gpu, int n_threads) {
    whisper_log_set(quiet_log, NULL);
    struct ghi_wh * w = (struct ghi_wh *) calloc(1, sizeof *w);
    if (!w) return NULL;
    w->n_threads = n_threads > 0 ? n_threads : 4;
    strcpy(w->last_allowed, "vi");

    struct whisper_context_params cp = whisper_context_default_params();
    cp.use_gpu = use_gpu != 0;
    // DTW token timestamps (word times) are not supported with flash attention.
    cp.flash_attn = false;
    cp.dtw_token_timestamps = true;
    cp.dtw_aheads_preset = WHISPER_AHEADS_LARGE_V3_TURBO;
    w->ctx = whisper_init_from_file_with_params(model, cp);

    struct whisper_vad_context_params vp = whisper_vad_default_context_params();
    vp.n_threads = w->n_threads;
    // The VAD net is tiny; the CPU is as fast and keeps it off the GPU queue.
    vp.use_gpu = false;
    w->vad = whisper_vad_init_from_file_with_params(vad_model, vp);

    if (!w->ctx || !w->vad) {
        if (w->ctx) whisper_free(w->ctx);
        if (w->vad) whisper_vad_free(w->vad);
        free(w);
        return NULL;
    }
    return w;
}

void ghi_wh_free(struct ghi_wh * w) {
    if (!w) return;
    if (w->vad_segments) whisper_vad_free_segments(w->vad_segments);
    whisper_vad_free(w->vad);
    whisper_free(w->ctx);
    free(w);
}

// Speech regions of `samples` (16 kHz mono). 0 on success; read them back with
// ghi_wh_vad_count / ghi_wh_vad_t0 / ghi_wh_vad_t1 (seconds).
int ghi_wh_vad(struct ghi_wh * w, const float * samples, int n, float threshold,
               int min_speech_ms, int min_silence_ms, float max_speech_s, int pad_ms) {
    if (w->vad_segments) {
        whisper_vad_free_segments(w->vad_segments);
        w->vad_segments = NULL;
    }
    struct whisper_vad_params p = whisper_vad_default_params();
    p.threshold = threshold;
    p.min_speech_duration_ms = min_speech_ms;
    p.min_silence_duration_ms = min_silence_ms;
    p.max_speech_duration_s = max_speech_s;
    p.speech_pad_ms = pad_ms;
    whisper_vad_reset_state(w->vad);
    w->vad_segments = whisper_vad_segments_from_samples(w->vad, p, samples, n);
    return w->vad_segments ? 0 : -1;
}

int ghi_wh_vad_count(struct ghi_wh * w) {
    return w->vad_segments ? whisper_vad_segments_n_segments(w->vad_segments) : 0;
}
// whisper.cpp reports VAD times in centiseconds.
double ghi_wh_vad_t0(struct ghi_wh * w, int i) {
    return whisper_vad_segments_get_segment_t0(w->vad_segments, i) / 100.0;
}
double ghi_wh_vad_t1(struct ghi_wh * w, int i) {
    return whisper_vad_segments_get_segment_t1(w->vad_segments, i) / 100.0;
}

static int run_full(struct ghi_wh * w, const float * samples, int n, const char * lang,
                    bool (*abort_cb)(void *), void * abort_ud) {
    struct whisper_full_params p = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
    p.n_threads = w->n_threads;
    p.print_progress = false;
    p.print_realtime = false;
    p.print_timestamps = false;
    p.print_special = false;
    p.translate = false;
    // Each group is its own window: no carried text, so a hallucinated loop in
    // one group cannot seed the next.
    p.no_context = true;
    p.suppress_blank = true;
    p.suppress_nst = true;
    p.token_timestamps = false; // DTW (context param) supplies the word times
    p.language = lang;
    p.detect_language = false;
    // Checked inside the encoder and the decoder loops: a recording that starts
    // mid-decode does not wait for the whole window.
    p.abort_callback = abort_cb;
    p.abort_callback_user_data = abort_ud;
    return whisper_full(w->ctx, p, samples, n);
}

static int is_allowed_lang(int id) {
    return id == whisper_lang_id("en") || id == whisper_lang_id("vi");
}

// Decodes one window of up to 30 s. `lang` is a whisper code ("vi", "en") or
// NULL/"auto". With auto, a language other than English or Vietnamese is
// re-decoded as whichever of the two scores higher (Ghira's languages; a short
// Vietnamese group is often misread as Thai or Indonesian); if that score
// cannot be had, as the last of the two used. `abort_cb(abort_ud)` returning
// true stops the decode (a nonzero result). 0 on success.
int ghi_wh_decode(struct ghi_wh * w, const float * samples, int n, const char * lang,
                  bool (*abort_cb)(void *), void * abort_ud) {
    const int is_auto = !lang || !*lang || strcmp(lang, "auto") == 0;
    int rc = run_full(w, samples, n, is_auto ? "auto" : lang, abort_cb, abort_ud);
    if (rc != 0) return rc;
    int id = whisper_full_lang_id(w->ctx);
    if (is_auto && !is_allowed_lang(id)) {
        const int en = whisper_lang_id("en"), vi = whisper_lang_id("vi");
        int pick = strcmp(w->last_allowed, "en") == 0 ? en : vi;
        float * probs = (float *) calloc((size_t) whisper_lang_max_id() + 1, sizeof(float));
        if (probs && whisper_pcm_to_mel(w->ctx, samples, n, w->n_threads) == 0 &&
            whisper_lang_auto_detect(w->ctx, 0, w->n_threads, probs) >= 0) {
            pick = probs[vi] > probs[en] ? vi : en;
        }
        free(probs);
        rc = run_full(w, samples, n, whisper_lang_str(pick), abort_cb, abort_ud);
        if (rc != 0) return rc;
    }
    const char * used = whisper_lang_str(whisper_full_lang_id(w->ctx));
    strncpy(w->lang, used, sizeof w->lang - 1);
    if (strcmp(used, "en") == 0 || strcmp(used, "vi") == 0) strcpy(w->last_allowed, used);
    return 0;
}

// The language the last decode used ("vi", "en", ...).
const char * ghi_wh_lang(struct ghi_wh * w) { return w->lang; }

int ghi_wh_seg_count(struct ghi_wh * w) { return whisper_full_n_segments(w->ctx); }
// Segment times in seconds from the start of the decoded window.
double ghi_wh_seg_t0(struct ghi_wh * w, int i) { return whisper_full_get_segment_t0(w->ctx, i) / 100.0; }
double ghi_wh_seg_t1(struct ghi_wh * w, int i) { return whisper_full_get_segment_t1(w->ctx, i) / 100.0; }
const char * ghi_wh_seg_text(struct ghi_wh * w, int i) { return whisper_full_get_segment_text(w->ctx, i); }
float ghi_wh_seg_no_speech(struct ghi_wh * w, int i) { return whisper_full_get_segment_no_speech_prob(w->ctx, i); }

int ghi_wh_tok_count(struct ghi_wh * w, int seg) { return whisper_full_n_tokens(w->ctx, seg); }
// Raw token bytes (a Vietnamese letter can be split across byte-fallback tokens).
const char * ghi_wh_tok_text(struct ghi_wh * w, int seg, int tok) {
    return whisper_full_get_token_text(w->ctx, seg, tok);
}
// 1 for text tokens, 0 for special ones (timestamps, language, end of text).
int ghi_wh_tok_is_text(struct ghi_wh * w, int seg, int tok) {
    return whisper_full_get_token_id(w->ctx, seg, tok) < whisper_token_eot(w->ctx);
}
float ghi_wh_tok_p(struct ghi_wh * w, int seg, int tok) { return whisper_full_get_token_p(w->ctx, seg, tok); }
// DTW time of the token in seconds from the window start; < 0 when unknown.
double ghi_wh_tok_dtw(struct ghi_wh * w, int seg, int tok) {
    const whisper_token_data d = whisper_full_get_token_data(w->ctx, seg, tok);
    return d.t_dtw < 0 ? -1.0 : d.t_dtw / 100.0;
}
