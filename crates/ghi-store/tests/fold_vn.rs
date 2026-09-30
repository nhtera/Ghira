// SPDX-License-Identifier: Apache-2.0
//! Vietnamese accent-insensitive search acceptance (doc 05 §2.1): folding and
//! end-to-end search with highlights on the original characters.

mod common;

use ghi_store::fold::{self, fold, fold_mapped};
use ghi_store::search::{SearchHit, SearchQuery};
use ghi_store::store::{NewNoteBlock, Provenance, Store};
use unicode_normalization::UnicodeNormalization;

fn hit_text(h: &SearchHit) -> Vec<String> {
    h.highlights
        .iter()
        .map(|r| {
            h.snippet
                .chars()
                .skip(r.start - h.snippet_start)
                .take(r.len())
                .collect()
        })
        .collect()
}

fn search(store: &Store, q: &str) -> Vec<SearchHit> {
    store.search(&SearchQuery::new(q)).unwrap()
}

#[test]
fn fold_matches_doc_05_examples() {
    assert_eq!(fold("Đồng"), "dong");
    assert_eq!(fold("đồng"), "dong");
    assert_eq!(fold("Đà Nẵng"), "da nang");
    assert_eq!(fold("chốt"), "chot");
    assert_eq!(fold("họp"), "hop");
    assert_eq!(fold("ĐỒNG ĐÀ NẴNG"), "dong da nang");
    assert_eq!(fold("Hello, World"), "hello, world");
}

#[test]
fn fold_keeps_one_char_per_char_for_vietnamese() {
    let sample = "Nguyễn Thị Ánh ở Đà Nẵng chốt họp lúc 9 giờ, đồng ý — “Được rồi”";
    let nfc: String = sample.nfc().collect();
    let f = fold_mapped(sample);
    assert_eq!(f.text.chars().count(), nfc.chars().count());
    assert!(f.map.is_none(), "1:1 folding needs no offset map");
    // Composed and decomposed input fold identically.
    let nfd: String = sample.nfd().collect();
    assert_eq!(fold(&nfd), fold(sample));
    assert_eq!(fold::nfc(&nfd), nfc);
}

#[test]
fn all_vietnamese_letters_fold_to_ascii_one_to_one() {
    let letters = "aàáảãạăằắẳẵặâầấẩẫậeèéẻẽẹêềếểễệiìíỉĩịoòóỏõọôồốổỗộơờớởỡợuùúủũụưừứửữựyỳýỷỹỵđ";
    for c in letters.chars().chain(letters.to_uppercase().chars()) {
        let s = c.to_string();
        let f = fold_mapped(&s);
        assert_eq!(f.text.chars().count(), 1, "{c}");
        assert!(
            f.text.chars().all(|x| x.is_ascii_lowercase()),
            "{c} -> {}",
            f.text
        );
    }
}

#[test]
fn search_acceptance_cases() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "Họp kế hoạch");
    store
        .add_segments(
            &m,
            vec![
                common::seg(0, 3000, "Tỷ giá đồng hôm nay tăng nhẹ."),
                common::seg(3000, 6000, "Chuyến công tác Đà Nẵng tuần sau."),
                common::seg(6000, 9000, "Cả nhóm chốt phương án này nhé."),
                common::seg(9000, 12000, "Mai họp lúc chín giờ."),
                common::seg(12000, 15000, "Nothing to see here, plain English."),
            ],
        )
        .unwrap();

    for (q, expect, word) in [
        ("dong", "Tỷ giá đồng hôm nay tăng nhẹ.", "đồng"),
        ("da nang", "Chuyến công tác Đà Nẵng tuần sau.", "Đà Nẵng"),
        ("chot", "Cả nhóm chốt phương án này nhé.", "chốt"),
        ("hop", "Mai họp lúc chín giờ.", "họp"),
        ("Đồng", "Tỷ giá đồng hôm nay tăng nhẹ.", "đồng"),
        ("ĐÀ NẴNG", "Chuyến công tác Đà Nẵng tuần sau.", "Đà Nẵng"),
        ("CHỐT", "Cả nhóm chốt phương án này nhé.", "chốt"),
    ] {
        let hits = search(&store, q);
        let seg_hits: Vec<_> = hits.iter().filter(|h| h.snippet == expect).collect();
        assert_eq!(seg_hits.len(), 1, "query {q:?}: {hits:#?}");
        assert_eq!(hit_text(seg_hits[0]), [word], "query {q:?}");
    }
}

#[test]
fn decomposed_input_is_searchable_and_highlights_use_stored_offsets() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    // macOS-style decomposed text and query.
    let text: String = "Họp ở Đà Nẵng, đồng ý".nfd().collect();
    store.add_segment(&m, common::seg(0, 1000, &text)).unwrap();
    let q: String = "đồng".nfd().collect();
    let hits = search(&store, &q);
    assert_eq!(hits.len(), 1);
    // The stored text is NFC, so offsets are NFC char offsets.
    assert_eq!(hits[0].snippet, fold::nfc(&text));
    assert_eq!(hit_text(&hits[0]), ["đồng"]);
    assert_eq!(hits[0].highlights[0], 15..19);
}

#[test]
fn exact_diacritics_rank_first_when_the_query_has_them() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    store
        .add_segments(
            &m,
            vec![
                // "đông" (east) and "đống" (heap) fold to "dong" like "đồng".
                common::seg(0, 1000, "Mùa đông năm nay lạnh"),
                common::seg(1000, 2000, "Một đống việc cần làm"),
                common::seg(2000, 3000, "Tỷ giá đồng tăng"),
                common::seg(3000, 4000, "dong dong dong dong"),
            ],
        )
        .unwrap();
    let plain = search(&store, "dong");
    assert_eq!(plain.len(), 4, "accent-less query finds every form");
    assert!(plain.iter().all(|h| !h.exact));

    let accented = search(&store, "đồng");
    assert_eq!(accented.len(), 4, "still finds the others");
    assert_eq!(accented[0].snippet, "Tỷ giá đồng tăng", "exact form first");
    assert!(accented[0].exact);
    assert!(accented[1..].iter().all(|h| !h.exact));
}

#[test]
fn prefix_search_and_multi_word() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    store
        .add_segments(
            &m,
            vec![
                common::seg(0, 1, "Ngân hàng nhà nước công bố"),
                common::seg(1, 2, "Ngân sách quý bốn"),
            ],
        )
        .unwrap();
    assert_eq!(search(&store, "ngan").len(), 2, "prefix of the last word");
    assert_eq!(search(&store, "ngan hang").len(), 1);
    assert_eq!(
        search(&store, "hang ngan").len(),
        1,
        "word order does not matter"
    );
    assert!(search(&store, "xyz").is_empty());
}

#[test]
fn notes_are_searched_and_highlighted_too() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    store
        .add_note_block(
            &m,
            NewNoteBlock {
                kind: "paragraph".into(),
                provenance: Provenance::User,
                body: "Quyết định: chốt ngân sách".into(),
                anchors: vec![],
                pinned: false,
            },
        )
        .unwrap();
    let hits = search(&store, "chot");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].kind, ghi_store::search::HitKind::Note);
    assert_eq!(hit_text(&hits[0]), ["chốt"]);
}
