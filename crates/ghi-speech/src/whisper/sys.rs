// SPDX-License-Identifier: Apache-2.0
//! Bindings for `shim.c`, the flat C face over the pinned whisper.cpp (RT-15:
//! upstream changes stay in the shim). Everything here is plain ints, floats
//! and pointers, so there are no struct layouts to keep in step.

use std::ffi::{c_char, c_double, c_float, c_int, c_void};

#[repr(C)]
pub struct Ctx {
    _private: [u8; 0],
}

unsafe extern "C" {
    pub fn ghi_wh_version() -> *const c_char;
    pub fn ghi_wh_new(
        model: *const c_char,
        vad_model: *const c_char,
        use_gpu: c_int,
        n_threads: c_int,
    ) -> *mut Ctx;
    pub fn ghi_wh_free(w: *mut Ctx);

    pub fn ghi_wh_vad(
        w: *mut Ctx,
        samples: *const c_float,
        n: c_int,
        threshold: c_float,
        min_speech_ms: c_int,
        min_silence_ms: c_int,
        max_speech_s: c_float,
        pad_ms: c_int,
    ) -> c_int;
    pub fn ghi_wh_vad_count(w: *mut Ctx) -> c_int;
    pub fn ghi_wh_vad_t0(w: *mut Ctx, i: c_int) -> c_double;
    pub fn ghi_wh_vad_t1(w: *mut Ctx, i: c_int) -> c_double;

    pub fn ghi_wh_decode(
        w: *mut Ctx,
        samples: *const c_float,
        n: c_int,
        lang: *const c_char,
        abort_cb: Option<unsafe extern "C" fn(*mut c_void) -> bool>,
        abort_ud: *mut c_void,
    ) -> c_int;
    pub fn ghi_wh_lang(w: *mut Ctx) -> *const c_char;
    pub fn ghi_wh_seg_count(w: *mut Ctx) -> c_int;
    pub fn ghi_wh_seg_t0(w: *mut Ctx, i: c_int) -> c_double;
    pub fn ghi_wh_seg_t1(w: *mut Ctx, i: c_int) -> c_double;
    pub fn ghi_wh_seg_text(w: *mut Ctx, i: c_int) -> *const c_char;
    pub fn ghi_wh_seg_no_speech(w: *mut Ctx, i: c_int) -> c_float;
    pub fn ghi_wh_tok_count(w: *mut Ctx, seg: c_int) -> c_int;
    pub fn ghi_wh_tok_text(w: *mut Ctx, seg: c_int, tok: c_int) -> *const c_char;
    pub fn ghi_wh_tok_is_text(w: *mut Ctx, seg: c_int, tok: c_int) -> c_int;
    pub fn ghi_wh_tok_p(w: *mut Ctx, seg: c_int, tok: c_int) -> c_float;
    pub fn ghi_wh_tok_dtw(w: *mut Ctx, seg: c_int, tok: c_int) -> c_double;
}
