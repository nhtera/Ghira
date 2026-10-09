// SPDX-License-Identifier: Apache-2.0
//! Search behaviour (filters, pagination, safety, index upkeep) and the
//! 1,000-meeting latency bench (< 100 ms p95 in release).

mod common;

use std::sync::Mutex;
use std::time::Instant;

use ghi_store::search::{HitKind, SearchFilter, SearchQuery};
use ghi_store::store::{NewMeeting, NewNoteBlock, NewSegment, NewSpeaker, Provenance, Store};

/// Tests share the CPU with the timing bench; run them one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|p| p.into_inner())
}

#[test]
fn bundled_sqlite_supports_contentless_delete() {
    let _g = serial();
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    // The schema needs contentless_delete (SQLite >= 3.43); opening proved it.
    let v = store.sqlite_version().unwrap();
    let mut parts = v.split('.').map(|p| p.parse::<u32>().unwrap());
    let (major, minor) = (parts.next().unwrap(), parts.next().unwrap());
    assert!(major > 3 || (major == 3 && minor >= 43), "SQLite {v}");
}

/// A proposed decision is a note block like any other: found by search.
#[test]
fn proposals_and_saved_answers_are_searchable_like_decisions() {
    let _g = serial();
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "Họp");
    for (kind, body) in [
        ("decision", "Ship the beta on Friday"),
        ("proposal", "Maybe add a dark theme"),
        ("answer", "Q: Who owns QA?\nA: Nam handles testing."),
    ] {
        store
            .add_note_block(
                &m,
                NewNoteBlock {
                    kind: kind.into(),
                    provenance: Provenance::Ai,
                    body: body.into(),
                    anchors: vec![],
                    pinned: false,
                },
            )
            .unwrap();
    }
    for (q, want) in [
        ("beta", "Ship the beta"),
        ("dark theme", "Maybe add a dark"),
    ] {
        let hits = store.search(&SearchQuery::new(q)).unwrap();
        assert_eq!(hits.len(), 1, "{q}");
        assert_eq!(hits[0].kind, HitKind::Note);
        assert!(hits[0].snippet.starts_with(want), "{}", hits[0].snippet);
    }
}

fn two_meetings(store: &Store) -> (String, String, String) {
    let alice = store.add_person("Alice", 1).unwrap();
    let m1 = store
        .create_meeting(NewMeeting {
            title: "Họp sản phẩm".into(),
            started_at: 1_000,
            source: "live".into(),
            template: Some("standup".into()),
            ..Default::default()
        })
        .unwrap()
        .gid;
    let m2 = store
        .create_meeting(NewMeeting {
            title: "Phỏng vấn".into(),
            started_at: 5_000,
            source: "file".into(),
            template: Some("interview".into()),
            ..Default::default()
        })
        .unwrap()
        .gid;
    let sp = store
        .add_speaker(
            &m1,
            NewSpeaker {
                label_idx: 0,
                person_gid: Some(alice.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    let sp2 = store
        .add_speaker(
            &m1,
            NewSpeaker {
                label_idx: 1,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .add_segments(
            &m1,
            vec![
                NewSegment {
                    speaker_gid: Some(sp),
                    ..common::seg(0, 1000, "Ngân sách quý này đã chốt")
                },
                NewSegment {
                    speaker_gid: Some(sp2),
                    ..common::seg(1000, 2000, "Ngân sách chưa chốt")
                },
            ],
        )
        .unwrap();
    store
        .add_segment(
            &m2,
            common::seg(0, 1000, "Ngân hàng đã chốt lịch phỏng vấn"),
        )
        .unwrap();
    (alice, m1, m2)
}

#[test]
fn filters_narrow_the_results() {
    let _g = serial();
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let (alice, m1, m2) = two_meetings(&store);
    let run = |f: SearchFilter| {
        store
            .search(&SearchQuery {
                text: "chot".into(),
                filter: f,
                limit: 20,
                offset: 0,
            })
            .unwrap()
    };

    assert_eq!(run(SearchFilter::default()).len(), 3);
    assert_eq!(
        run(SearchFilter {
            source: Some("file".into()),
            ..Default::default()
        })[0]
            .meeting_gid,
        m2
    );
    assert_eq!(
        run(SearchFilter {
            source: Some("live".into()),
            ..Default::default()
        })
        .len(),
        2
    );
    assert_eq!(
        run(SearchFilter {
            template: Some("interview".into()),
            ..Default::default()
        })
        .len(),
        1
    );
    assert_eq!(
        run(SearchFilter {
            from_ms: Some(2_000),
            ..Default::default()
        })
        .len(),
        1
    );
    assert_eq!(
        run(SearchFilter {
            to_ms: Some(2_000),
            ..Default::default()
        })
        .len(),
        2
    );
    assert_eq!(
        run(SearchFilter {
            meeting_gid: Some(m1.clone()),
            ..Default::default()
        })
        .len(),
        2
    );
    assert_eq!(
        run(SearchFilter {
            meeting_gids: vec![m1.clone(), m2.clone()],
            ..Default::default()
        })
        .len(),
        3
    );
    assert_eq!(
        run(SearchFilter {
            meeting_gids: vec![m2.clone()],
            ..Default::default()
        })[0]
            .meeting_gid,
        m2
    );
    assert!(
        run(SearchFilter {
            segments_only: true,
            ..Default::default()
        })
        .iter()
        .all(|h| h.kind == HitKind::Segment)
    );
    let by_person = run(SearchFilter {
        person_gids: vec![alice],
        ..Default::default()
    });
    assert_eq!(by_person.len(), 1, "only Alice's segment");
    assert_eq!(by_person[0].snippet, "Ngân sách quý này đã chốt");
    assert_eq!(by_person[0].meeting_title, "Họp sản phẩm");
    assert!(
        run(SearchFilter {
            person_gids: vec!["nobody".into()],
            ..Default::default()
        })
        .is_empty()
    );
}

#[test]
fn pagination_pages_do_not_overlap() {
    let _g = serial();
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    let segs = (0..25)
        .map(|i| common::seg(i * 10, i * 10 + 9, &format!("từ khóa số {i} lặp lại")))
        .collect();
    store.add_segments(&m, segs).unwrap();

    let page = |offset| {
        store
            .search(&SearchQuery {
                text: "khoa".into(),
                filter: Default::default(),
                limit: 10,
                offset,
            })
            .unwrap()
    };
    let (p1, p2, p3, p4) = (page(0), page(10), page(20), page(30));
    assert_eq!((p1.len(), p2.len(), p3.len(), p4.len()), (10, 10, 5, 0));
    let mut all: Vec<_> = p1
        .iter()
        .chain(&p2)
        .chain(&p3)
        .map(|h| h.item_gid.clone())
        .collect();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), 25);
    assert!(
        store
            .search(&SearchQuery {
                limit: 0,
                ..SearchQuery::new("khoa")
            })
            .unwrap()
            .is_empty()
    );
}

#[test]
fn hostile_queries_are_just_words() {
    let _g = serial();
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    store
        .add_segment(
            &m,
            common::seg(0, 1, "AND OR NOT NEAR column text_norm: star*"),
        )
        .unwrap();
    for q in [
        "\"",
        "\"\"\"",
        "*",
        "a AND",
        "NEAR(",
        "text_norm:foo",
        "{text_norm}: x",
        "^x",
        "x -y",
        "( ) ( )",
        "'; DROP TABLE meetings;--",
        "\u{0}",
        "   ",
        "",
        "%",
        "-",
        "OR OR OR",
    ] {
        store
            .search(&SearchQuery::new(q))
            .unwrap_or_else(|e| panic!("query {q:?} failed: {e}"));
    }
    // Operator words are plain words.
    assert_eq!(store.search(&SearchQuery::new("near")).unwrap().len(), 1);
    assert_eq!(
        store.search(&SearchQuery::new("and or not")).unwrap().len(),
        1
    );
    assert_eq!(store.list_meetings(10, 0).unwrap().len(), 1);
}

#[test]
fn edits_and_replaced_transcripts_update_the_index() {
    let _g = serial();
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    let live = store
        .add_segment(&m, common::seg(0, 2000, "kế hoạch tạm thời"))
        .unwrap();

    assert_eq!(
        store.search(&SearchQuery::new("tam thoi")).unwrap().len(),
        1
    );
    store
        .update_segment_text(&live.gid, "kế hoạch chính thức")
        .unwrap();
    assert!(
        store
            .search(&SearchQuery::new("tam thoi"))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.search(&SearchQuery::new("chinh thuc")).unwrap().len(),
        1
    );

    // The final pass replaces the live transcript wholesale.
    let v = store
        .replace_transcript(&m, vec![common::seg(0, 2000, "bản cuối cùng")])
        .unwrap();
    assert_eq!(v, 2);
    assert!(
        store
            .search(&SearchQuery::new("chinh thuc"))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.search(&SearchQuery::new("cuoi cung")).unwrap().len(),
        1
    );
    let segs = store.segments(&m).unwrap();
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].version, 2);
    assert_eq!(store.get_meeting(&m).unwrap().transcript_version, 2);
    // Old segment gids are tombstoned for sync.
    assert!(store.is_tombstoned(&live.gid).unwrap());
}

#[test]
fn deleted_meetings_are_not_found() {
    let _g = serial();
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let (_alice, m1, _m2) = two_meetings(&store);
    assert_eq!(store.search(&SearchQuery::new("chot")).unwrap().len(), 3);
    store.delete_meeting(&m1).unwrap();
    let hits = store.search(&SearchQuery::new("chot")).unwrap();
    assert_eq!(hits.len(), 1);
    assert_ne!(hits[0].meeting_gid, m1);
    assert!(hits.iter().all(|h| h.kind == HitKind::Segment));
}

// ---------------------------------------------------------------- the bench

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    /// Zipf-ish: low indexes are much more common.
    fn skewed(&mut self, n: usize) -> usize {
        let a = self.below(n);
        let b = self.below(n);
        a.min(b).min(self.below(n))
    }
}

const VN: &[&str] = &[
    "họp",
    "đồng",
    "chốt",
    "ngân",
    "sách",
    "kế",
    "hoạch",
    "quý",
    "khách",
    "hàng",
    "dự",
    "án",
    "tiến",
    "độ",
    "báo",
    "cáo",
    "Đà",
    "Nẵng",
    "Hà",
    "Nội",
    "công",
    "ty",
    "sản",
    "phẩm",
    "thị",
    "trường",
    "giá",
    "thành",
    "chi",
    "phí",
    "nhân",
    "sự",
    "tuyển",
    "dụng",
    "phỏng",
    "vấn",
    "hợp",
    "đàm",
    "phán",
    "ký",
    "kết",
    "triển",
    "khai",
    "kiểm",
    "thử",
    "lỗi",
    "sửa",
    "phát",
    "hành",
    "phiên",
    "bản",
    "mới",
    "người",
    "dùng",
    "phản",
    "hồi",
    "cải",
    "thiện",
    "hiệu",
    "suất",
    "bảo",
    "mật",
];
const EN: &[&str] = &[
    "meeting",
    "budget",
    "roadmap",
    "release",
    "customer",
    "feedback",
    "priority",
    "deadline",
    "review",
    "design",
    "launch",
    "quarter",
    "revenue",
    "hiring",
    "planning",
    "sprint",
    "backlog",
    "metrics",
    "onboarding",
    "security",
    "contract",
    "vendor",
    "pricing",
    "forecast",
    "decision",
    "action",
    "owner",
    "risk",
    "scope",
    "update",
    "the",
    "and",
    "we",
    "should",
    "ship",
    "next",
    "week",
    "team",
    "product",
    "engineering",
    "support",
    "growth",
    "retention",
];
const FILLER_VN: &[&str] = &[
    "là", "và", "của", "cho", "này", "rồi", "nhé", "thì", "được", "một",
];

fn sentence(rng: &mut Rng, vn: bool) -> String {
    let n = 8 + rng.below(8);
    let mut words: Vec<&str> = Vec::with_capacity(n);
    for _ in 0..n {
        let w = if vn {
            if rng.below(3) == 0 {
                FILLER_VN[rng.below(FILLER_VN.len())]
            } else {
                VN[rng.skewed(VN.len())]
            }
        } else {
            EN[rng.skewed(EN.len())]
        };
        words.push(w);
    }
    let mut s = words.join(" ");
    s.push('.');
    s
}

fn percentile(sorted_ms: &[f64], p: f64) -> f64 {
    let idx = ((sorted_ms.len() as f64) * p).ceil() as usize;
    sorted_ms[idx.clamp(1, sorted_ms.len()) - 1]
}

#[test]
fn p95_under_100ms_on_1000_meetings() {
    let _g = serial();
    const MEETINGS: usize = 1_000;
    const SEGMENTS: usize = 50;
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);

    let t = Instant::now();
    for i in 0..MEETINGS {
        let vn = i % 3 != 2;
        let m = store
            .create_meeting(NewMeeting {
                title: format!("Cuộc họp {i}"),
                started_at: 1_600_000_000_000 + (i as i64) * 86_400_000,
                source: if i % 10 == 0 { "file" } else { "live" }.into(),
                template: Some(["standup", "interview", "planning"][i % 3].into()),
                ..Default::default()
            })
            .unwrap();
        let segs: Vec<NewSegment> = (0..SEGMENTS)
            .map(|k| {
                common::seg(
                    (k as i64) * 4000,
                    (k as i64) * 4000 + 3900,
                    &sentence(&mut rng, vn),
                )
            })
            .collect();
        store.add_segments(&m.gid, segs).unwrap();
    }
    let build = t.elapsed();

    // Realistic queries: accent-less VN, accented VN, EN, phrases, prefixes, rare words, filtered.
    let mut queries: Vec<(String, SearchFilter)> = Vec::new();
    for q in [
        "hop",
        "dong",
        "chot",
        "ngan sach",
        "ke hoach",
        "da nang",
        "ha noi",
        "khach hang",
        "du an tien do",
        "bao cao",
        "họp",
        "đồng",
        "chốt",
        "ngân sách",
        "kế hoạch",
        "Đà Nẵng",
        "phản hồi",
        "bảo mật",
        "tuyen dung",
        "phong van",
        "meeting",
        "budget",
        "roadmap release",
        "customer feedback",
        "priority",
        "deadline",
        "we should ship",
        "hiring plan",
        "sec",
        "ngan",
        "kiem thu loi",
        "sua loi",
        "phien ban moi",
        "khong co tu nay",
        "zzzz",
        "the",
        "và",
        "hop dong",
        "chi phi",
        "thi truong gia",
    ] {
        queries.push((q.to_string(), SearchFilter::default()));
    }
    for (i, q) in ["hop", "budget", "ngan sach", "chot", "customer"]
        .iter()
        .enumerate()
    {
        queries.push((
            q.to_string(),
            SearchFilter {
                source: Some("live".into()),
                ..Default::default()
            },
        ));
        queries.push((
            q.to_string(),
            SearchFilter {
                from_ms: Some(1_600_000_000_000 + 200 * 86_400_000),
                to_ms: Some(1_600_000_000_000 + (400 + i as i64 * 50) * 86_400_000),
                template: Some("planning".into()),
                ..Default::default()
            },
        ));
    }
    assert!(queries.len() >= 50);

    // Warm up (statement cache, page cache, DEK cache).
    for (q, f) in queries.iter().take(4) {
        store
            .search(&SearchQuery {
                text: q.clone(),
                filter: f.clone(),
                limit: 20,
                offset: 0,
            })
            .unwrap();
    }
    let mut samples = Vec::new();
    let mut hits_total = 0;
    for _round in 0..3 {
        for (q, f) in &queries {
            let start = Instant::now();
            let hits = store
                .search(&SearchQuery {
                    text: q.clone(),
                    filter: f.clone(),
                    limit: 20,
                    offset: 0,
                })
                .unwrap();
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            hits_total += hits.len();
        }
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    let (p50, p95, max) = (
        percentile(&samples, 0.50),
        percentile(&samples, 0.95),
        *samples.last().unwrap(),
    );
    println!(
        "search bench: {MEETINGS} meetings x {SEGMENTS} segments, build {build:.1?}, {} queries x3: \
         p50 {p50:.2} ms, p95 {p95:.2} ms, max {max:.2} ms ({hits_total} hits total, profile {})",
        queries.len(),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    assert!(hits_total > 0);

    // Cost of the compaction every delete runs (FTS optimize + vacuum + checkpoint).
    let t = Instant::now();
    store.compact_index().unwrap();
    let compact = t.elapsed();
    let victim = store.list_meetings(1, 0).unwrap().remove(0).gid;
    let t = Instant::now();
    store.delete_meeting(&victim).unwrap();
    println!(
        "compact_index {compact:.1?}; delete_meeting (incl. FTS compaction + key rotation over ~1000 meetings) {:.1?}",
        t.elapsed()
    );
    #[cfg(not(debug_assertions))]
    assert!(p95 < 100.0, "p95 {p95:.2} ms");
}

#[test]
fn pages_are_stable_beyond_the_old_window_and_with_exact_tiers() {
    let _g = serial();
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    // 260 hits; every third one carries the exact accented form.
    let segs: Vec<NewSegment> = (0..260)
        .map(|i| {
            let word = if i % 3 == 0 { "đồng" } else { "dong" };
            common::seg(i * 10, i * 10 + 9, &format!("mục {i} {word} lặp"))
        })
        .collect();
    store.add_segments(&m, segs).unwrap();

    for query in ["dong", "đồng"] {
        let mut seen = std::collections::HashSet::new();
        let mut order = Vec::new();
        for page in 0..6 {
            let p = store
                .search_page(&SearchQuery {
                    text: query.into(),
                    filter: Default::default(),
                    limit: 50,
                    offset: page * 50,
                })
                .unwrap();
            assert!(!p.truncated, "260 < window");
            for h in p.hits {
                assert!(seen.insert(h.item_gid.clone()), "duplicate across pages");
                order.push(h.exact);
            }
        }
        assert_eq!(seen.len(), 260, "no hit skipped for {query:?}");
        if query == "đồng" {
            // All exact matches come first, then the rest.
            let first_plain = order.iter().position(|e| !e).unwrap();
            assert_eq!(first_plain, 87);
            assert!(order[first_plain..].iter().all(|e| !e));
        }
    }
}
