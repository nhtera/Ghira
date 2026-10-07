// SPDX-License-Identifier: Apache-2.0
//! Real-model check of the embedding worker: skipped when the model file or
//! the worker binary is missing, so CI without models passes.

use std::path::PathBuf;

use ghi_llm::embed::{Embedder, Kind, LocalEmbedder, cosine};
use ghi_llm::sidecar::worker_path;

fn open() -> Option<LocalEmbedder> {
    let dir = std::env::var_os("GHI_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models"));
    let model = ghi_models::find("qwen3-embedding-0.6b")?;
    if !ghi_models::path_in(&dir, &model).is_file() {
        eprintln!("skip: qwen3-embedding-0.6b model file not found");
        return None;
    }
    if !cfg!(feature = "inproc") && worker_path().is_err() {
        eprintln!("skip: ghi-llm-worker binary not built (cargo build -p ghi-llm-worker)");
        return None;
    }
    Some(LocalEmbedder::open_registry_in(&dir, "qwen3-embedding-0.6b").expect("model loads"))
}

#[test]
fn vietnamese_paraphrase_scores_above_unrelated_text() {
    let Some(mut e) = open() else { return };
    assert_eq!(e.dim(), 1024);
    let docs: Vec<String> = [
        "Chúng ta cần chốt ngân sách marketing cho quý bốn trước thứ Sáu.",
        "Phải hoàn tất kinh phí quảng cáo của quý cuối năm trước ngày thứ sáu.",
        "Hôm qua tôi đi chợ mua cá và rau để nấu canh chua.",
    ]
    .map(String::from)
    .to_vec();
    let v = e.embed(&docs, Kind::Document).unwrap();
    for x in &v {
        assert!((cosine(x, x) - 1.0).abs() < 1e-3, "unit length");
    }
    let (para, other) = (cosine(&v[0], &v[1]), cosine(&v[0], &v[2]));
    eprintln!("cos(paraphrase) = {para:.4}, cos(unrelated) = {other:.4}");
    assert!(para > other + 0.1);

    // A query finds its passage across languages.
    let q = e
        .embed(
            &["When is the Q4 marketing budget due?".into()],
            Kind::Query,
        )
        .unwrap();
    let (hit, miss) = (cosine(&q[0], &v[0]), cosine(&q[0], &v[2]));
    eprintln!("cos(query, budget) = {hit:.4}, cos(query, market) = {miss:.4}");
    assert!(hit > miss);
}
