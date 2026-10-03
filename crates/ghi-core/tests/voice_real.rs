// SPDX-License-Identifier: Apache-2.0
//! The voice step's cost with the real speaker model (feature `voice`; skips
//! without the model or the eval audio). Run in release for the timings:
//! `cargo test -p ghi-core --features voice --release --test voice_real -- --ignored --nocapture`.
#![cfg(feature = "voice")]

use std::path::{Path, PathBuf};
use std::time::Instant;

use ghi_core::profiles::{Span, cosine, embed_windows, normalized, open_tract, pick_windows};

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

const MODEL: &str = "models/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx";
const AMI: &str = "tools/eval/data/ami-sdm";

/// RTTM speaker turns as (label, span); the label's index among the sorted
/// labels is the `who` of its spans.
fn rttm(meeting: &str) -> Option<(Vec<String>, Vec<Span>)> {
    let text =
        std::fs::read_to_string(root().join(AMI).join(format!("labels/{meeting}.rttm"))).ok()?;
    let rows: Vec<(String, i64, i64)> = text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() >= 8 && f[0] == "SPEAKER").then(|| {
                let t0 = (f[3].parse::<f64>().unwrap() * 1000.0) as i64;
                let dur = (f[4].parse::<f64>().unwrap() * 1000.0) as i64;
                (f[7].to_string(), t0, t0 + dur)
            })
        })
        .collect();
    let mut labels: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
    labels.sort();
    labels.dedup();
    let spans = rows
        .into_iter()
        .map(|(l, t0_ms, t1_ms)| Span {
            who: labels.iter().position(|x| *x == l).unwrap() as u32,
            t0_ms,
            t1_ms,
        })
        .collect();
    Some((labels, spans))
}

/// One speaker's voice in one AMI meeting, by our real window policy.
fn ami_voice(
    emb: &mut dyn ghi_core::profiles::VoiceEmbed,
    meeting: &str,
    label: &str,
) -> Option<Vec<f32>> {
    let pcm = wav(&format!("{AMI}/audio/{meeting}.wav"))?;
    let (labels, spans) = rttm(meeting)?;
    let who = labels.iter().position(|l| l == label)? as u32;
    let w = pick_windows(&spans, who, pcm.len());
    embed_windows(emb, &pcm, &w, &|| false)
        .ok()??
        .vec
        .clone()
        .into()
}

#[test]
#[ignore = "needs the speaker model and AMI audio (tools/eval/scripts/fetch_public_sets.py --sets ami-sdm --subset full)"]
fn ami_same_speaker_scores_above_different_speaker() {
    // ES2004: the same four people in every meeting of the series.
    let model = root().join(MODEL);
    if !model.exists() || !root().join(AMI).join("audio/ES2004b.wav").exists() {
        eprintln!("skipped: no speaker model or AMI audio");
        return;
    }
    let mut emb = open_tract(&model).unwrap();
    let (labels, _) = rttm("ES2004b").unwrap();
    let (a, b) = (&labels[0], &labels[1]);
    let a_in_b = ami_voice(emb.as_mut(), "ES2004b", a).unwrap();
    let a_in_c = ami_voice(emb.as_mut(), "ES2004c", a).unwrap();
    let b_in_b = ami_voice(emb.as_mut(), "ES2004b", b).unwrap();
    let b_in_c = ami_voice(emb.as_mut(), "ES2004c", b).unwrap();
    let (same_a, same_b) = (cosine(&a_in_b, &a_in_c), cosine(&b_in_b, &b_in_c));
    let diff = [
        cosine(&a_in_b, &b_in_c),
        cosine(&b_in_b, &a_in_c),
        cosine(&a_in_b, &b_in_b),
        cosine(&a_in_c, &b_in_c),
    ];
    eprintln!("same: {same_a:.3} {same_b:.3}; different: {diff:.3?}");
    for d in diff {
        assert!(
            same_a > d && same_b > d,
            "same {same_a}/{same_b} vs different {d}"
        );
    }
}

/// Writes every AMI speaker's per-meeting voice vector (unit, our window
/// policy) to `tools/eval/data/ami-sdm/voices.json` for
/// `ghi-eval speakerid-report`. Run in release:
/// `cargo test -p ghi-core --features voice --release --test voice_real -- --ignored ami_dump_voices`.
#[test]
#[ignore = "needs the speaker model and AMI audio; writes voices.json into the git-ignored eval data dir"]
fn ami_dump_voices() {
    let model = root().join(MODEL);
    let dir = root().join(AMI).join("labels");
    if !model.exists() || !dir.exists() {
        eprintln!("skipped: no speaker model or AMI data");
        return;
    }
    let mut emb = open_tract(&model).unwrap();
    let mut meetings: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let n = e.ok()?.file_name().into_string().ok()?;
            n.strip_suffix(".rttm").map(str::to_string)
        })
        .filter(|m| root().join(AMI).join(format!("audio/{m}.wav")).exists())
        .collect();
    meetings.sort();
    let mut out = serde_json::Map::new();
    for m in &meetings {
        let pcm = wav(&format!("{AMI}/audio/{m}.wav")).unwrap();
        let (labels, spans) = rttm(m).unwrap();
        let mut per = serde_json::Map::new();
        for (i, l) in labels.iter().enumerate() {
            let w = pick_windows(&spans, i as u32, pcm.len());
            if let Some(v) = embed_windows(emb.as_mut(), &pcm, &w, &|| false).unwrap() {
                per.insert(
                    l.clone(),
                    serde_json::json!({"vec": v.vec.clone(), "windows": v.windows, "speech_s": v.speech as f64 / 16_000.0}),
                );
            }
        }
        eprintln!("{m}: {} speakers", per.len());
        out.insert(m.clone(), per.into());
    }
    std::fs::write(
        root().join(AMI).join("voices.json"),
        serde_json::to_string(&out).unwrap(),
    )
    .unwrap();
}
