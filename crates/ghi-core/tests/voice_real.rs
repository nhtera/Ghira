// SPDX-License-Identifier: Apache-2.0
//! The voice step's cost with the real speaker model (feature `voice`; skips
//! without the model or the eval audio). Run in release for the timings:
//! `cargo test -p ghi-core --features voice --release --test voice_real -- --ignored --nocapture`.
#![cfg(feature = "voice")]

use std::path::{Path, PathBuf};
use std::time::Instant;

use ghi_core::profiles::{cosine, embed_windows, normalized, open_tract};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn wav(rel: &str) -> Option<Vec<f32>> {
    let mut r = hound::WavReader::open(root().join(rel)).ok()?;
    let spec = r.spec();
    assert_eq!((spec.sample_rate, spec.channels), (16_000, 1));
    Some(
        r.samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / 32_768.0)
            .collect(),
    )
}

#[test]
#[ignore = "needs the speaker model and eval audio; run explicitly with --ignored (see the header)"]
#[allow(clippy::single_range_in_vec_init)]
fn real_model_timings_and_sanity() {
    let model = root().join("models/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx");
    let (Some(a), Some(b)) = (
        wav("tools/eval/data/voxconverse/audio/msbyq.wav"),
        wav("tools/eval/data/fleurs-vi/audio/fleurs_10259532564282505455.wav"),
    ) else {
        eprintln!("skipped: no eval audio");
        return;
    };
    if !model.exists() {
        eprintln!("skipped: no speaker model");
        return;
    }
    let t = Instant::now();
    let mut emb = open_tract(&model).unwrap();
    eprintln!("open: {:?}", t.elapsed());

    // The same audio is the same voice; the vector is finite, unit after normalising.
    let w = [0..97_600usize];
    let v1 = embed_windows(emb.as_mut(), &a, &w, &|| false)
        .unwrap()
        .unwrap();
    let v2 = embed_windows(emb.as_mut(), &a, &w, &|| false)
        .unwrap()
        .unwrap();
    assert!(cosine(&v1.vec, &v2.vec) > 0.9999);
    let vb = embed_windows(emb.as_mut(), &b, &[0..65_600], &|| false)
        .unwrap()
        .unwrap();
    eprintln!("cos(other recording) = {:.3}", cosine(&v1.vec, &vb.vec));
    assert!(normalized(&v1.vec).is_some());

    // A 30-minute meeting with 5 speakers: 5 clusters x 8 windows x 6.1 s,
    // plus Me's 8 windows (call mode).
    let windows: Vec<std::ops::Range<usize>> =
        (0..8).map(|i| i * 97_600..(i + 1) * 97_600).collect();
    let mut long = Vec::new();
    while long.len() < 8 * 97_600 {
        long.extend_from_slice(&a);
    }
    let t = Instant::now();
    for _ in 0..6 {
        let v = embed_windows(emb.as_mut(), &long, &windows, &|| false).unwrap();
        assert!(v.is_some());
    }
    eprintln!(
        "voice step, 30 min, 5 speakers + Me: {:?} (48 windows of 6.1 s)",
        t.elapsed()
    );
    let t = Instant::now();
    let _ = emb.embed(&long[..33_600]).unwrap();
    eprintln!("one 2.1 s window: {:?}", t.elapsed());
}
