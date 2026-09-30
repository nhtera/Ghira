// SPDX-License-Identifier: Apache-2.0
//! Lexical retrieval over one meeting's segments (BM25 on folded tokens, the
//! same folding as the store's FTS index), for Ask and enhance. Semantic
//! neighbours join in with embeddings (phase 14, hybrid search).

use std::collections::HashMap;

use crate::transcript::Transcript;

const K1: f64 = 1.2;
const B: f64 = 0.75;

pub struct Index {
    /// Folded tokens per segment (same order as `Transcript::segments`).
    docs: Vec<Vec<String>>,
    df: HashMap<String, usize>,
    avg_len: f64,
}

impl Index {
    pub fn new(t: &Transcript) -> Index {
        let docs: Vec<Vec<String>> = t.segments().iter().map(|s| terms(&s.text)).collect();
        let mut df: HashMap<String, usize> = HashMap::new();
        for d in &docs {
            let mut seen: Vec<&String> = d.iter().collect();
            seen.sort_unstable();
            seen.dedup();
            for term in seen {
                *df.entry(term.clone()).or_default() += 1;
            }
        }
        let total: usize = docs.iter().map(Vec::len).sum();
        let avg_len = total as f64 / docs.len().max(1) as f64;
        Index { docs, df, avg_len }
    }

    /// Indexes into `Transcript::segments` of the best `k` matches for `query`
    /// (score > 0), best first.
    pub fn search(&self, query: &str, k: usize) -> Vec<usize> {
        let q = query_terms(query);
        if q.is_empty() {
            return Vec::new();
        }
        let n = self.docs.len() as f64;
        let mut scored: Vec<(usize, f64)> = self
            .docs
            .iter()
            .enumerate()
            .filter_map(|(i, d)| {
                let len = d.len() as f64;
                let score: f64 = q
                    .iter()
                    .map(|term| {
                        let tf = d.iter().filter(|t| *t == term).count() as f64;
                        if tf == 0.0 {
                            return 0.0;
                        }
                        let df = *self.df.get(term).unwrap_or(&0) as f64;
                        let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                        idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * len / self.avg_len))
                    })
                    .sum();
                (score > 0.0).then_some((i, score))
            })
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        scored.truncate(k);
        scored.into_iter().map(|(i, _)| i).collect()
    }
}

fn terms(text: &str) -> Vec<String> {
    ghi_text::tokens(&ghi_text::fold(text))
        .into_iter()
        .map(|(_, t)| t)
        .collect()
}

/// The distinct folded terms a query is searched with (shown to the user
/// when the answer is "not discussed").
pub fn query_terms(query: &str) -> Vec<String> {
    let mut q = terms(query);
    q.sort_unstable();
    q.dedup();
    q
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::tests::seg;

    #[test]
    fn finds_accent_insensitive_matches_best_first() {
        let t = Transcript::new(vec![
            seg(0, 0.0, 1.0, "S1", "Hôm nay họp về ngân sách quý ba", "vi"),
            seg(
                1,
                1.0,
                2.0,
                "S2",
                "Ngân sách marketing tăng, ngân sách IT giữ nguyên",
                "vi",
            ),
            seg(2, 2.0, 3.0, "S1", "Chốt lịch ra mắt ở Đà Nẵng", "vi"),
        ])
        .unwrap();
        let ix = Index::new(&t);
        assert_eq!(ix.search("ngan sach", 5), vec![1, 0]);
        assert_eq!(ix.search("da nang", 5), vec![2]);
        assert!(ix.search("calendar", 5).is_empty());
        assert!(ix.search("  ", 5).is_empty());
    }
}
