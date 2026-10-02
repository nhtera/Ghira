// SPDX-License-Identifier: Apache-2.0
//! Voice embedder parity against fixtures made by
//! `tests/fixtures/make_voice_fixtures.py` (kaldi-native-fbank + onnxruntime).
//! The embedding tests need the model file and skip without it.
#![cfg(feature = "voice")]

use std::path::PathBuf;

use ghi_speech::voice::fbank::{self, Fbank, NUM_MEL};
use ghi_speech::voice::{TractVoice, VoiceEmbedder};

const N: usize = 97_000;

/// Integer-only synthetic signal, identical to the script's `signal`.
fn signal(n: usize) -> Vec<f32> {
    let mut s: i64 = 12345;
    (0..n as i64)
        .map(|i| {
            s = (s * 1_103_515_245 + 12345) & 0x7FFF_FFFF;
            let noise = ((s >> 16) & 0x7FF) - 1024;
            let tri1 = ((i * 37) % 200 - 100).abs() * 60 - 3000;
            let tri2 = ((i * 11) % 64 - 32).abs() * 90 - 1440;
            (noise + tri1 + tri2) as i16 as f32 / 32768.0
        })
        .collect()
}

fn fixture(name: &str) -> Vec<f32> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&p)
        .unwrap()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}

fn model_path() -> Option<PathBuf> {
    let dir = std::env::var_os("GHI_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"));
    let p = dir.join("3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx");
    p.is_file().then_some(p)
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let n = |v: &[f32]| v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    dot / (n(a) * n(b))
}

fn max_abs_err(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

#[test]
fn fbank_matches_kaldi_reference() {
    let x = signal(N);
    let frames = Fbank::new().compute(&x);
    assert_eq!(frames.len(), 604 * NUM_MEL);
    let n = 20 * NUM_MEL;
    let head = max_abs_err(&frames[..n], &fixture("fbank_head.f32"));
    let tail = max_abs_err(&frames[frames.len() - n..], &fixture("fbank_tail.f32"));
    let mut mean = vec![0.0f32; NUM_MEL];
    for row in frames.chunks_exact(NUM_MEL) {
        for (m, v) in mean.iter_mut().zip(row) {
            *m += v / 604.0;
        }
    }
    let mean_err = max_abs_err(&mean, &fixture("fbank_mean.f32"));
    eprintln!("fbank max abs err: head {head} tail {tail} mean {mean_err}");
    assert!(head <= 1e-3, "head max abs err {head}");
    assert!(tail <= 1e-3, "tail max abs err {tail}");
    assert!(mean_err <= 1e-3, "mean max abs err {mean_err}");
}

#[test]
fn embedder_matches_onnxruntime() {
    let Some(path) = model_path() else {
        eprintln!(
            "skip: campplus model not installed (tools/scripts/fetch-models.sh campplus-zh-en)"
        );
        return;
    };
    let mut voice = TractVoice::open(&path).unwrap();
    let x = signal(N);
    // Exact bucket sizes, plus 32 400 samples (201 frames -> the 200 bucket)
    // and the full signal (604 frames -> the 600 bucket).
    let at = fbank::samples_for_frames;
    for (pcm, name) in [
        (&x[..at(200)], "emb_200.f32"),
        (&x[..32_400], "emb_200.f32"),
        (&x[..at(400)], "emb_400.f32"),
        (&x[..at(600)], "emb_600.f32"),
        (&x[..], "emb_600.f32"),
    ] {
        let got = voice.embed(pcm).unwrap().expect("long enough");
        let want = fixture(name);
        let cos = cosine(&got, &want);
        eprintln!("{name}: cos {cos}");
        assert!(cos >= 0.99999, "{name}: cos {cos}");
    }
}

#[test]
fn buckets_short_and_silent_audio() {
    let Some(path) = model_path() else {
        eprintln!("skip: campplus model not installed");
        return;
    };
    let mut voice = TractVoice::open(&path).unwrap();
    let x = signal(N);
    // 199 frames (just under 2.015 s): too short.
    assert_eq!(
        voice.embed(&x[..fbank::samples_for_frames(199)]).unwrap(),
        None
    );
    assert_eq!(voice.embed(&[]).unwrap(), None);
    // Anything from 200 to 399 frames is the 200 bucket: extra tail is ignored.
    let a = voice
        .embed(&x[..fbank::samples_for_frames(200)])
        .unwrap()
        .unwrap();
    let b = voice
        .embed(&x[..fbank::samples_for_frames(399)])
        .unwrap()
        .unwrap();
    assert_eq!(a.len(), 192);
    assert_eq!(a, b);
    // 600 frames and above crop to the 600 bucket.
    let long = signal(fbank::samples_for_frames(650));
    let c = voice.embed(&long).unwrap().unwrap();
    let d = voice
        .embed(&long[..fbank::samples_for_frames(600)])
        .unwrap()
        .unwrap();
    assert_eq!(c, d);

    // Silence and very quiet noise (RMS ~3e-4) are not embedded; speech-like is.
    assert_eq!(voice.embed(&vec![0.0; 40_000]).unwrap(), None);
    let quiet: Vec<f32> = x.iter().map(|v| v * 0.005).collect();
    assert_eq!(voice.embed(&quiet).unwrap(), None);
    assert!(voice.embed(&x).unwrap().is_some());
    // Non-finite input is an error, not NaN.
    let mut bad = x[..fbank::samples_for_frames(200)].to_vec();
    bad[100] = f32::NAN;
    assert!(voice.embed(&bad).is_err());
    bad[100] = f32::INFINITY;
    assert!(voice.embed(&bad).is_err());

    for (t, pcm) in [(200, 200), (400, 400), (600, 600)]
        .map(|(t, f)| (t, &long[..fbank::samples_for_frames(f)]))
    {
        let t0 = std::time::Instant::now();
        voice.embed(pcm).unwrap();
        eprintln!("embed {t} frames: {:?}", t0.elapsed());
    }
}
