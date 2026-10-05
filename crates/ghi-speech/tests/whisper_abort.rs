// SPDX-License-Identifier: Apache-2.0
//! A decode stops promptly when told to (a recording preempting the final pass).
//! Real models; skips without them or the FLEURS-vi clips.
//!
//!     cargo test -p ghi-speech --features whisper --test whisper_abort -- --ignored --nocapture
#![cfg(feature = "whisper")]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ghi_speech::AsrStream;
use ghi_speech::whisper::{Whisper, WhisperConfig, WhisperOptions};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every FLEURS-vi clip in a row (PCM16 mono 16 kHz, 44-byte header).
fn audio() -> Option<Vec<f32>> {
    let mut files: Vec<_> = std::fs::read_dir(root().join("tools/eval/data/fleurs-vi/audio"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    let mut pcm = Vec::new();
    for f in files {
        let bytes = std::fs::read(f).ok()?;
        pcm.extend(
            bytes[44..]
                .chunks_exact(2)
                .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0),
        );
    }
    (!pcm.is_empty()).then_some(pcm)
}

#[test]
#[ignore = "needs the real models and FLEURS-vi (tools/scripts/fetch-models.sh)"]
fn abort_stops_a_decode_quickly() {
    let models = std::env::var_os("GHI_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("models"));
    let model = models.join("ggml-large-v3-turbo-q5_0.bin");
    let vad_model = models.join("ggml-silero-v6.2.0.bin");
    let Some(pcm) = audio() else {
        eprintln!("skipped: no FLEURS-vi clips");
        return;
    };
    if !model.is_file() || !vad_model.is_file() {
        eprintln!("skipped: models missing in {}", models.display());
        return;
    }
    let whisper = Arc::new(
        Whisper::new(&WhisperConfig {
            model,
            vad_model,
            gpu: true,
            threads: 0,
        })
        .unwrap(),
    );

    let mut stream = whisper.stream_owned(&WhisperOptions::default());
    stream.push(&pcm, 16_000).unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let setter = {
        let flag = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            flag.store(true, Ordering::SeqCst);
        })
    };
    let started = Instant::now();
    let finished = stream
        .finish_abortable(&|| flag.load(Ordering::SeqCst))
        .unwrap();
    let took = started.elapsed();
    setter.join().unwrap();
    println!(
        "aborted after {took:?} (audio {:.0} s)",
        pcm.len() as f64 / 16e3
    );
    assert!(!finished, "the decode finished before the abort");
    assert!(took < Duration::from_secs(4), "abort took {took:?}");
    assert!(
        stream.next_result().unwrap().is_none(),
        "no partial results"
    );

    // The same stream can decode again afterwards.
    assert!(stream.finish_abortable(&|| false).unwrap());
    assert!(stream.next_result().unwrap().is_some());
}
