// SPDX-License-Identifier: Apache-2.0
//! Phase 7 WER gate for the glossary packs. Packs act on the final pass's
//! lines, so this reads each eval clip once with the final-pass engine
//! (NeMo, 1120 ms chunks, language auto), then scores the same lines with the
//! packs off and with each pack configuration applied the way `final_pass`
//! does (per line, `Vocabulary::pack`). Prints the corpus WER per set and every
//! line a pack changed.
//!
//!     cargo test -p ghi-core --features nemo --release --test pack_wer \
//!       -- --ignored --nocapture
//! knobs: GHI_PACK_WER_SETS=fleurs-en,fleurs-vi,vimedcss  GHI_PACK_WER_CLIPS=<max per set>
#![cfg(feature = "nemo")]

use std::path::{Path, PathBuf};

use ghi_core::engines::{NemoEngines, SpeechEngines};
use ghi_core::vocab::{Vocabulary, pack_terms};
use ghi_speech::nemo::Device;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_wav(path: &Path) -> Vec<f32> {
    let mut d = ghi_audio::decode::Decoder::open(path)
        .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
    let mut pcm = Vec::new();
    while let Some(b) = d.next_block().unwrap() {
        let n = b.channels.len() as f32;
        pcm.extend((0..b.frames()).map(|i| b.channels.iter().map(|c| c[i]).sum::<f32>() / n));
    }
    pcm
}

/// Lowercase words without punctuation.
fn words(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Word edits between a reference and a hypothesis.
fn edits(reference: &str, hyp: &str) -> usize {
    let (r, h) = (words(reference), words(hyp));
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1; h.len() + 1];
        for (j, hw) in h.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(rw != hw))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[h.len()]
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
#[ignore = "needs NeMo (tools/scripts/build-nemo.sh), the speech models and the eval sets"]
fn pack_wer_gate() {
    let models = std::env::var_os("GHI_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("models"));
    let (asr, diar) = (
        models.join("nemotron-3.5-asr-streaming-0.6b.q8_0.gguf"),
        models.join("Nemotron-3-Diarization.q8_0.gguf"),
    );
    if !asr.exists() || !diar.exists() {
        eprintln!("skipped: models missing in {}", models.display());
        return;
    }
    let engines = NemoEngines::load(&asr, &diar, 1120, Device::Gpu).unwrap();
    let want = std::env::var("GHI_PACK_WER_SETS")
        .unwrap_or_else(|_| "fleurs-en,fleurs-vi,vimedcss".into());
    let max: usize = std::env::var("GHI_PACK_WER_CLIPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    // (label, vocabulary) per configuration.
    let configs: Vec<(&str, Vocabulary)> = vec![
        (
            "tech+finance",
            Vocabulary::pack(&pack_terms(&ids(&[
                "tech-en",
                "tech-vi",
                "finance-en",
                "finance-vi",
            ]))),
        ),
        (
            "medical-vi",
            Vocabulary::pack(&pack_terms(&ids(&["medical-vi"]))),
        ),
        (
            "medical-en",
            Vocabulary::pack(&pack_terms(&ids(&["medical-en"]))),
        ),
        (
            "all 8",
            Vocabulary::pack(&pack_terms(&ids(&[
                "medical-en",
                "medical-vi",
                "legal-en",
                "legal-vi",
                "finance-en",
                "finance-vi",
                "tech-en",
                "tech-vi",
            ]))),
        ),
    ];
    for set in want.split(',').map(str::trim) {
        let dir = root().join("tools/eval/data").join(set);
        let Ok(rd) = std::fs::read_dir(dir.join("refs")) else {
            eprintln!("skipped {set}: no data");
            continue;
        };
        let mut clips: Vec<String> = rd
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|n| n.strip_suffix(".txt").map(str::to_string))
            .collect();
        clips.sort();
        clips.truncate(max);
        let (mut ref_words, mut base) = (0usize, 0usize);
        let mut with = vec![0usize; configs.len()];
        let mut changed: Vec<Vec<String>> = vec![Vec::new(); configs.len()];
        let t = std::time::Instant::now();
        for id in &clips {
            let audio = std::fs::read_dir(dir.join("audio"))
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .find(|p| p.file_stem().is_some_and(|s| s == id.as_str()))
                .unwrap();
            let reference = std::fs::read_to_string(dir.join(format!("refs/{id}.txt"))).unwrap();
            let pcm = read_wav(&audio);
            let mut a = engines.asr(None).unwrap();
            for block in pcm.chunks(16_000) {
                a.push(block, 16_000).unwrap();
            }
            a.finish().unwrap();
            let mut lines: Vec<String> = Vec::new();
            while let Some(r) = a.next_result().unwrap() {
                if r.is_final && !r.text.trim().is_empty() {
                    lines.push(r.text);
                }
            }
            ref_words += words(&reference).len();
            base += edits(&reference, &lines.join(" "));
            for (k, (_, v)) in configs.iter().enumerate() {
                let fixed: Vec<String> = lines
                    .iter()
                    .map(|l| {
                        let c = v.correct(l);
                        if let Some(c) = &c {
                            changed[k].push(format!("{id}: `{l}` -> `{c}`"));
                        }
                        c.unwrap_or_else(|| l.clone())
                    })
                    .collect();
                with[k] += edits(&reference, &fixed.join(" "));
            }
        }
        let pct = |e: usize| 100.0 * e as f64 / ref_words.max(1) as f64;
        println!(
            "PACKWER {set}: {} clips, {ref_words} ref words, {:.1} s | off {:.2}%",
            clips.len(),
            t.elapsed().as_secs_f64(),
            pct(base)
        );
        for (k, (label, _)) in configs.iter().enumerate() {
            println!(
                "PACKWER {set}: {label:<13} {:.2}% (delta {:+.2} pt, {} lines changed)",
                pct(with[k]),
                pct(with[k]) - pct(base),
                changed[k].len()
            );
            for c in &changed[k] {
                println!("PACKWER   {label}: {c}");
            }
        }
    }
}
