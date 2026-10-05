// SPDX-License-Identifier: Apache-2.0
//! NeMo and Whisper loaded in ONE process (real models): the static whisper.cpp
//! ggml must not collide with NeMo's patched, dynamic ggml. Runs a NeMo
//! transcription, a Whisper one, then NeMo again and diarization after Whisper
//! has used the GPU. Skips without the models or the FLEURS-vi clips.
//!
//!     cargo test -p ghi-core --features nemo,whisper --test whisper_real \
//!       -- --ignored --nocapture
#![cfg(all(feature = "nemo", feature = "whisper"))]

use std::path::PathBuf;

use ghi_core::engines::{NemoEngines, SpeechEngines, WhisperFinalEngines};
use ghi_speech::nemo::Device;

fn models() -> PathBuf {
    std::env::var_os("GHI_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

fn clip() -> Option<Vec<f32>> {
    let dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/eval/data/fleurs-vi/audio");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    let mut r = hound::WavReader::open(files.first()?).ok()?;
    assert_eq!(r.spec().sample_rate, 16_000);
    Some(
        r.samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / 32768.0)
            .collect(),
    )
}

fn text_of(e: &dyn SpeechEngines, pcm: &[f32]) -> String {
    let mut asr = e.asr(Some("vi-VN")).unwrap();
    for block in pcm.chunks(16_000) {
        asr.push(block, 16_000).unwrap();
    }
    asr.finish().unwrap();
    let mut out = String::new();
    while let Some(r) = asr.next_result().unwrap() {
        if r.is_final {
            out.push_str(&r.text);
            out.push(' ');
        }
    }
    out
}

#[test]
#[ignore = "needs the real models and FLEURS-vi (tools/scripts/fetch-models.sh)"]
fn nemo_and_whisper_share_one_process() {
    let m = models();
    let f = |n: &str| m.join(n);
    let (asr, diar) = (
        f("nemotron-3.5-asr-streaming-0.6b.q8_0.gguf"),
        f("Nemotron-3-Diarization.q8_0.gguf"),
    );
    let (wm, vad) = (
        f("ggml-large-v3-turbo-q5_0.bin"),
        f("ggml-silero-v6.2.0.bin"),
    );
    let Some(pcm) = clip() else {
        eprintln!("skipped: no FLEURS-vi clip");
        return;
    };
    if ![&asr, &diar, &wm, &vad].iter().all(|p| p.is_file()) {
        eprintln!("skipped: models missing in {}", m.display());
        return;
    }

    let nemo = NemoEngines::load(&asr, &diar, 1120, Device::Gpu).unwrap();
    let before = text_of(&nemo, &pcm);
    assert!(!before.trim().is_empty(), "NeMo heard nothing");

    // Whisper loads while NeMo's models are still resident.
    let whisper = WhisperFinalEngines::load(&wm, &vad, &diar, 1120, Device::Gpu).unwrap();
    let w = text_of(&whisper, &pcm);
    assert!(!w.trim().is_empty(), "Whisper heard nothing");
    println!("nemo:    {before}\nwhisper: {w}");

    // Both still work after the other has run.
    assert_eq!(text_of(&nemo, &pcm).trim(), before.trim());
    for e in [&nemo as &dyn SpeechEngines, &whisper] {
        let mut d = e.diar().unwrap();
        for block in pcm.chunks(16_000) {
            d.push(block, 16_000).unwrap();
        }
        d.finish().unwrap();
        assert!(!d.segments().unwrap().is_empty(), "no speaker turns");
    }
}
