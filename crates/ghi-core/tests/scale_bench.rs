// SPDX-License-Identifier: Apache-2.0
//! Scale bench: a store of synthetic meetings (not shipped; run by hand).
//!
//! ```sh
//! GHI_BENCH_MEETINGS=1000 cargo test --release -p ghi-core --test scale_bench \
//!     -- --ignored --nocapture
//! ```
//!
//! Seeds ~60 one-minute chunks per meeting (EN and Vietnamese text, speakers,
//! notes, sealed embeddings) and prints p50/p95 of the library, search, ask
//! retrieval, people, detail and startup-recover paths. Synthetic content
//! only; nothing is logged but timings.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ghi_core::ask_all::{Scope, retrieve};
use ghi_llm::embed::{Embedder, Kind};
use ghi_store::embeddings::EmbeddingChunk;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::search::SearchQuery;
use ghi_store::store::{NewMeeting, NewNoteBlock, NewSegment, NewSpeaker, Provenance, Store};

const DIM: usize = 1024;
const MODEL: &str = "bench-embed";
const CHUNKS: usize = 60;
const LINES_PER_CHUNK: usize = 4;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.next() as usize % xs.len()]
    }
    fn unit_vec(&mut self) -> Vec<f32> {
        let mut v: Vec<f32> = (0..DIM)
            .map(|_| (self.next() % 2001) as f32 / 1000.0 - 1.0)
            .collect();
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter_mut().for_each(|x| *x /= n);
        v
    }
}

const EN: &[&str] = &[
    "budget",
    "roadmap",
    "launch",
    "customer",
    "pricing",
    "hiring",
    "contract",
    "review",
    "deadline",
    "feedback",
    "design",
    "release",
    "support",
    "forecast",
    "quarter",
    "vendor",
    "security",
    "migration",
    "pilot",
    "onboarding",
    "metrics",
    "partner",
    "invoice",
    "sprint",
];
const VI: &[&str] = &[
    "ngân", "sách", "khách", "hàng", "triển", "khai", "quyết", "định", "tuyển", "dụng", "hợp",
    "đồng", "đánh", "giá", "thời", "hạn", "phản", "hồi", "báo", "cáo", "kế", "hoạch", "đối", "tác",
];
const NAMES: &[&str] = &[
    "Minh", "Lan", "Hùng", "Trang", "Đức", "Mai", "Nam", "Thảo", "Quân", "Hà", "Linh", "Phúc",
    "An", "Bình", "Châu", "Dung", "Giang", "Hải", "Khoa", "Loan", "Nhung", "Oanh", "Phong",
    "Quỳnh", "Sơn", "Tâm", "Uyên", "Việt", "Xuân", "Yến",
];

fn sentence(r: &mut Rng, vn: bool) -> String {
    let pool = if vn { VI } else { EN };
    let n = 8 + r.next() as usize % 8;
    (0..n).map(|_| r.pick(pool)).collect::<Vec<_>>().join(" ")
}

/// Answers every query with one fixed vector (a stored chunk's, so it hits).
struct FixedEmbedder(Vec<f32>);

impl Embedder for FixedEmbedder {
    fn embed(&mut self, texts: &[String], _: Kind) -> ghi_llm::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| self.0.clone()).collect())
    }
    fn dim(&self) -> usize {
        DIM
    }
    fn model_id(&self) -> &str {
        MODEL
    }
}

fn report(name: &str, mut ms: Vec<f64>) {
    ms.sort_by(f64::total_cmp);
    let at = |p: f64| ms[(((ms.len() - 1) as f64) * p).round() as usize];
    println!(
        "BENCH {name:<34} n={:<3} p50={:>8.1} ms  p95={:>8.1} ms  max={:>8.1} ms",
        ms.len(),
        at(0.5),
        at(0.95),
        ms[ms.len() - 1]
    );
}

fn time<T>(runs: usize, mut f: impl FnMut(usize) -> T) -> Vec<f64> {
    (0..runs)
        .map(|i| {
            let t = Instant::now();
            std::hint::black_box(f(i));
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect()
}

#[test]
#[ignore = "scale bench; run by hand"]
fn scale_bench() {
    let n: usize = std::env::var("GHI_BENCH_MEETINGS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    let mut r = Rng(42);
    let seed_start = Instant::now();
    let mut gids = Vec::with_capacity(n);
    let mut probe = Vec::new();
    for i in 0..n {
        let vn = i % 3 == 0;
        let m = store
            .create_meeting(NewMeeting {
                title: format!(
                    "{} {i}",
                    if vn {
                        "Họp kế hoạch"
                    } else {
                        "Planning sync"
                    }
                ),
                started_at: 1_700_000_000_000 + i as i64 * 3_600_000,
                source: "live".into(),
                mode: "call".into(),
                ..Default::default()
            })
            .unwrap();
        let sp: Vec<String> = (0..4)
            .map(|k| {
                store
                    .add_speaker(
                        &m.gid,
                        NewSpeaker {
                            label_idx: k,
                            display_name: (k < 2).then(|| {
                                if k == 0 && i % 5 == 0 {
                                    "Me".to_string()
                                } else {
                                    NAMES[(i + k as usize * 7) % NAMES.len()].to_string()
                                }
                            }),
                            person_gid: None,
                            color_slot: k + 1,
                            is_me: k == 0 && i % 5 == 0,
                        },
                    )
                    .unwrap()
            })
            .collect();
        let mut segs = Vec::with_capacity(CHUNKS * LINES_PER_CHUNK);
        for c in 0..CHUNKS {
            for l in 0..LINES_PER_CHUNK {
                let t0 = (c * 60_000 + l * 15_000) as i64;
                segs.push(NewSegment {
                    speaker_gid: Some(sp[r.next() as usize % 4].clone()),
                    t0_ms: t0,
                    t1_ms: t0 + 14_000,
                    text: sentence(&mut r, vn),
                    lang: Some(if vn { "vi" } else { "en" }.into()),
                    ..Default::default()
                });
            }
        }
        store.add_segments(&m.gid, segs).unwrap();
        let mut blocks = vec![("tldr", sentence(&mut r, vn))];
        for k in 0..12 {
            blocks.push((
                if k < 6 { "topic" } else { "decision" },
                sentence(&mut r, vn),
            ));
        }
        let blocks = blocks
            .into_iter()
            .map(|(kind, body)| NewNoteBlock {
                kind: kind.into(),
                provenance: Provenance::Ai,
                body,
                anchors: Vec::new(),
                pinned: false,
            })
            .collect();
        store.replace_ai_notes(&m.gid, blocks, Vec::new()).unwrap();
        store.set_meeting_status(&m.gid, "ready").unwrap();
        let v = store.get_meeting(&m.gid).unwrap().transcript_version;
        let chunks: Vec<EmbeddingChunk> = (0..CHUNKS)
            .map(|c| EmbeddingChunk {
                chunk: c as u32,
                t0_ms: c as i64 * 60_000,
                t1_ms: (c as i64 + 1) * 60_000,
                vec: r.unit_vec(),
            })
            .collect();
        if i == n / 2 {
            probe = chunks[7].vec.clone();
        }
        store
            .put_embeddings(&m.gid, MODEL, v, store.index_gen(&m.gid).unwrap(), chunks)
            .unwrap();
        gids.push(m.gid);
    }
    store.link_named_speakers().unwrap();
    println!(
        "BENCH seeded {n} meetings ({} lines, {} vectors) in {:.1} s",
        n * CHUNKS * LINES_PER_CHUNK,
        n * CHUNKS,
        seed_start.elapsed().as_secs_f32()
    );

    // Library pages: the row queries behind list_meetings (50 per page).
    let pages = (n / 50).max(1);
    report(
        "library page (50 rows)",
        time(pages, |p| {
            let ms = store.list_meetings(50, p * 50).unwrap();
            let g: Vec<String> = ms.iter().map(|m| m.gid.clone()).collect();
            store.active_jobs().unwrap();
            store.speaker_chips(&g).unwrap();
            store.meeting_tags(&g).unwrap();
            store.first_tldrs(&g).unwrap();
            store.unnamed_voice_counts(&g).unwrap();
        }),
    );
    let page: Vec<String> = gids.iter().rev().take(50).cloned().collect();
    report(
        "  list_meetings only",
        time(pages, |p| store.list_meetings(50, p * 50).unwrap()),
    );
    report(
        "  first_tldrs (summary)",
        time(20, |_| store.first_tldrs(&page).unwrap()),
    );
    report(
        "  unnamed_voice_counts",
        time(20, |_| store.unnamed_voice_counts(&page).unwrap()),
    );
    report(
        "  speaker_chips",
        time(20, |_| store.speaker_chips(&page).unwrap()),
    );

    // Search.
    let queries = [
        "budget roadmap",
        "ngân sách",
        "khách hàng",
        "deadline",
        "hợp đồng đánh giá",
    ];
    report(
        "FTS search (page of 20)",
        time(30, |i| {
            store
                .search(&SearchQuery::new(queries[i % queries.len()]))
                .unwrap()
        }),
    );
    let scope = Scope::default();
    // First query after a launch or unlock: the cache is empty, then the app
    // warms it off the UI path (`Core::warm_vectors`) and the user searches.
    store.clear_embedding_cache();
    let t = Instant::now();
    let cold = {
        let mut e = FixedEmbedder(probe.clone());
        retrieve(&store, Some(&mut e), queries[0], &scope, 12).unwrap();
        t.elapsed()
    };
    store.clear_embedding_cache();
    let t = Instant::now();
    let warmed = store
        .warm_embedding_index(MODEL, &|| true, &|| true)
        .unwrap();
    let warm_took = t.elapsed();
    let t = Instant::now();
    {
        let mut e = FixedEmbedder(probe.clone());
        retrieve(&store, Some(&mut e), queries[0], &scope, 12).unwrap();
    }
    println!(
        "BENCH first hybrid query, cold cache: {:.0} ms; warm call ({warmed} meetings): {:.0} ms, then first query: {:.0} ms",
        cold.as_secs_f64() * 1e3,
        warm_took.as_secs_f64() * 1e3,
        t.elapsed().as_secs_f64() * 1e3,
    );
    report(
        "  embedding_index (stamp, warm)",
        time(6, |_| store.embedding_index(MODEL).unwrap()),
    );
    report(
        "hybrid RRF retrieve (FTS+vector)",
        time(7, |i| {
            let mut e = FixedEmbedder(probe.clone());
            retrieve(&store, Some(&mut e), queries[i % queries.len()], &scope, 12).unwrap()
        }),
    );
    report(
        "  vector scan only (no keywords)",
        time(7, |_| {
            let mut e = FixedEmbedder(probe.clone());
            retrieve(&store, Some(&mut e), "zzzzqq", &scope, 12).unwrap()
        }),
    );

    // People and detail.
    report(
        "people overview",
        time(20, |_| store.people_overview().unwrap()),
    );
    report(
        "meeting detail open",
        time(30, |i| {
            let g = &gids[(i * 31) % gids.len()];
            store.get_meeting(g).unwrap();
            store.segments(g).unwrap();
            store.speakers(g).unwrap();
            store.meeting_words(g).unwrap();
            store.note_blocks(g).unwrap();
            store.action_items(g).unwrap();
        }),
    );

    // Startup recover.
    report(
        "startup recover",
        time(5, |_| ghi_core::recover::recover(&store).unwrap()),
    );
    let _ = Duration::ZERO;
}
