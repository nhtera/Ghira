// SPDX-License-Identifier: Apache-2.0
//! v1 → v2 speaker carry-over [RT-2]: the final pass re-diarizes the whole
//! meeting, and its clusters must inherit what the user did live (names, Me,
//! "not a person", merges and splits).
//!
//! Live speakers (after following merges, so a merge is a must-link) and
//! final clusters are matched one-to-one by maximum total overlap in time
//! (Hungarian assignment), so a split pair (cannot-link) never lands on the
//! same cluster. A match counts only if it covers at least [`MIN_SHARE`] of
//! the cluster's speech; other clusters are new speakers for "Name your
//! speakers".

/// A stretch of speech by one speaker (milliseconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Turn {
    pub speaker: u32,
    pub t0_ms: i64,
    pub t1_ms: i64,
}

/// A match below this share of the final cluster's speech is not trusted.
pub const MIN_SHARE: f64 = 0.5;

#[derive(Debug, Clone, PartialEq)]
pub struct Carry {
    /// Final cluster → live speaker, for confident matches.
    pub matched: Vec<(u32, u32)>,
    /// Final clusters with no confident live match.
    pub unmatched: Vec<u32>,
}

fn overlap_ms(a: &Turn, b: &Turn) -> i64 {
    (a.t1_ms.min(b.t1_ms) - a.t0_ms.max(b.t0_ms)).max(0)
}

fn ids(turns: &[Turn]) -> Vec<u32> {
    let mut v: Vec<u32> = turns.iter().map(|t| t.speaker).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Matches `final_turns` clusters to `live_turns` speakers.
pub fn carry_over(live_turns: &[Turn], final_turns: &[Turn]) -> Carry {
    let live = ids(live_turns);
    let fin = ids(final_turns);
    // Overlap matrix (final × live), in ms.
    let mut w = vec![vec![0i64; live.len()]; fin.len()];
    for f in final_turns {
        let i = fin.binary_search(&f.speaker).unwrap();
        for l in live_turns {
            let j = live.binary_search(&l.speaker).unwrap();
            w[i][j] += overlap_ms(f, l);
        }
    }
    let assignment = max_assignment(&w);
    let mut matched = Vec::new();
    let mut unmatched = Vec::new();
    for (i, f) in fin.iter().enumerate() {
        let total: i64 = final_turns
            .iter()
            .filter(|t| t.speaker == *f)
            .map(|t| t.t1_ms - t.t0_ms)
            .sum();
        match assignment[i] {
            Some(j) if total > 0 && w[i][j] as f64 >= MIN_SHARE * total as f64 => {
                matched.push((*f, live[j]))
            }
            _ => unmatched.push(*f),
        }
    }
    Carry { matched, unmatched }
}

/// Maximum-weight one-to-one assignment of rows to columns (Hungarian method
/// on the negated, squared-up matrix). `None` for rows left unassigned.
pub fn max_assignment(w: &[Vec<i64>]) -> Vec<Option<usize>> {
    let rows = w.len();
    let cols = w.first().map_or(0, Vec::len);
    let n = rows.max(cols);
    if n == 0 {
        return vec![None; rows];
    }
    let max = w.iter().flatten().copied().max().unwrap_or(0);
    // Minimize cost = max - weight; padding cells cost `max` (weight 0).
    let cost = |i: usize, j: usize| -> i64 {
        if i < rows && j < cols {
            max - w[i][j]
        } else {
            max
        }
    };
    // Classic O(n^3) potentials implementation (1-based internals).
    let inf = i64::MAX / 4;
    let mut u = vec![0i64; n + 1];
    let mut v = vec![0i64; n + 1];
    let mut p = vec![0usize; n + 1];
    let mut way = vec![0usize; n + 1];
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0usize;
        let mut minv = vec![inf; n + 1];
        let mut used = vec![false; n + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = inf;
            let mut j1 = 0usize;
            for j in 1..=n {
                if !used[j] {
                    let cur = cost(i0 - 1, j - 1) - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=n {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut out = vec![None; rows];
    for j in 1..=n {
        let i = p[j];
        if i >= 1 && i <= rows && j <= cols && w[i - 1][j - 1] > 0 {
            out[i - 1] = Some(j - 1);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(speaker: u32, t0: i64, t1: i64) -> Turn {
        Turn {
            speaker,
            t0_ms: t0,
            t1_ms: t1,
        }
    }

    #[test]
    fn assignment_maximizes_total_overlap() {
        // Greedy would give row 0 → col 0 (9) and leave row 1 with 1;
        // optimal is 0 → 1 (8) + 1 → 0 (8) = 16.
        let w = vec![vec![9, 8], vec![8, 1]];
        assert_eq!(max_assignment(&w), vec![Some(1), Some(0)]);
        // Rectangular: more rows than columns.
        let w = vec![vec![5], vec![7], vec![0]];
        assert_eq!(max_assignment(&w), vec![None, Some(0), None]);
        assert_eq!(max_assignment(&[]), Vec::<Option<usize>>::new());
    }

    #[test]
    fn clusters_inherit_live_speakers_and_weak_matches_are_new() {
        // Live: 1 talks 0–10 s, 2 talks 10–20 s.
        let live = [turn(1, 0, 10_000), turn(2, 10_000, 20_000)];
        // Final: A (7) covers 0–9.5 s, B (9) covers 9.5–20 s, C (3) is a
        // short voice live never separated (only 30% inside speaker 2).
        let fin = [
            turn(7, 0, 9_500),
            turn(9, 9_500, 20_000),
            turn(3, 20_000, 21_000),
            turn(3, 15_000, 15_300),
        ];
        let c = carry_over(&live, &fin);
        assert_eq!(c.matched, vec![(7, 1), (9, 2)]);
        assert_eq!(c.unmatched, vec![3]);
    }

    #[test]
    fn a_split_pair_never_shares_a_cluster() {
        // The user split speaker 2 off speaker 1; the final pass found two
        // clusters with roughly the same boundary.
        let live = [turn(1, 0, 6_000), turn(2, 6_000, 10_000)];
        let fin = [turn(5, 0, 5_500), turn(6, 5_500, 10_000)];
        let c = carry_over(&live, &fin);
        assert_eq!(c.matched, vec![(5, 1), (6, 2)]);
    }
}
