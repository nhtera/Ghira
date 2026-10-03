// SPDX-License-Identifier: Apache-2.0
//! Speaker re-clustering without the diarizer's 8-speaker cap (phase 14d,
//! D12): when the final pass's diarizer saturates, diarize short windows
//! independently, embed each (window, label) with the speaker model and
//! cluster across windows (average-linkage on cosine). Vectors live in memory
//! only and are wiped (RT-13).
//!
//! Gated by [`ENABLED`]: it ships only when the VoxConverse eval beats the
//! capped result (`tools/eval/reports/`).

use ghi_speech::SpeakerSegment;
use std::collections::BTreeSet;

use zeroize::Zeroize;

use crate::engines::SpeechEngines;
use crate::final_pass::cut_points;
use crate::profiles::{Span, VoiceEmbed, embed_windows, mean_normalized, pick_windows};

/// Distinct labels at which the diarizer is saturated.
pub const SATURATED_AT: usize = 8;

/// The final pass calls [`run`] only when this is true. On the VoxConverse eval
/// (16 files, 9-21 speakers) it cut DER from 27.6% to 7.1% and the speaker-count
/// error from 5.1 to 3.2, and left 7-8 speaker files unchanged
/// (`tools/eval/reports/recluster-20261003.md`).
pub const ENABLED: bool = true;

const RATE: usize = 16_000;
/// Diarization window (cut at quiet points).
pub const WINDOW_S: f64 = 120.0;
/// Embedding windows per (window, speaker): the longest ones.
const WINDOWS_PER_ITEM: usize = 3;

/// Clustering knobs, tuned on VoxConverse (`tools/eval/reports/`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    /// Average-linkage stops merging below this cosine.
    pub threshold: f32,
    /// A cluster with less speech than this (s) joins its nearest neighbour.
    pub min_cluster_s: f64,
}

pub const PARAMS: Params = Params {
    threshold: 0.6,
    min_cluster_s: 8.0,
};

thread_local! {
    static WIPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Vectors wiped so far by this thread (a test hook for RT-13: every path out
/// of [`analyse`], early or not, must wipe what it embedded).
#[doc(hidden)]
pub fn wiped_vectors_this_thread() -> usize {
    WIPED.with(|w| w.get())
}

/// One (window, local label): its segments and, when it had enough clean
/// speech, its voice. The voice is wiped when the item drops.
struct Item {
    win: usize,
    vec: Option<Vec<f32>>,
    /// Segments on the whole recording's timeline.
    segs: Vec<(f64, f64)>,
}

impl Drop for Item {
    fn drop(&mut self) {
        if let Some(v) = self.vec.as_mut() {
            v.zeroize();
            WIPED.with(|w| w.set(w.get() + 1));
        }
    }
}

impl Item {
    fn seconds(&self) -> f64 {
        self.segs.iter().map(|(a, b)| b - a).sum()
    }
    fn start(&self) -> f64 {
        self.segs.iter().map(|s| s.0).fold(f64::INFINITY, f64::min)
    }
}

/// Every window diarized and embedded; cluster it with [`Diarized::assign`].
pub struct Diarized {
    items: Vec<Item>,
}

/// Diarizes `pcm` in [`WINDOW_S`] windows and embeds each (window, label).
/// `Ok(None)` when `stop` fires, or when the audio is a single window (there
/// is nothing to tell apart across windows).
pub fn analyse(
    pcm: &[f32],
    engines: &dyn SpeechEngines,
    embedder: &mut dyn VoiceEmbed,
    stop: &dyn Fn() -> bool,
) -> Result<Option<Diarized>, String> {
    let cuts = cut_points(pcm, WINDOW_S);
    if cuts.len() <= 2 {
        return Ok(None);
    }
    let mut items = Vec::new();
    for (win, w) in cuts.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        if b <= a {
            continue;
        }
        let slice = &pcm[a..b];
        let len_s = slice.len() as f64 / RATE as f64;
        let mut diar = engines.diar().map_err(|e| e.to_string())?;
        for block in slice.chunks(10 * RATE) {
            if stop() {
                return Ok(None);
            }
            diar.push(block, RATE as u32).map_err(|e| e.to_string())?;
        }
        diar.finish().map_err(|e| e.to_string())?;
        // Segments clamped to the window (a stream may run past its audio).
        let segs: Vec<SpeakerSegment> = diar
            .segments()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter_map(|s| {
                let (start, end) = (s.start.max(0.0), s.end.min(len_s));
                (end > start).then_some(SpeakerSegment { start, end, ..s })
            })
            .collect();
        let spans: Vec<Span> = segs
            .iter()
            .map(|s| Span {
                who: s.speaker,
                t0_ms: (s.start * 1000.0) as i64,
                t1_ms: (s.end * 1000.0) as i64,
            })
            .collect();
        let mut labels: Vec<u32> = spans.iter().map(|s| s.who).collect();
        labels.sort_unstable();
        labels.dedup();
        let offset = a as f64 / RATE as f64;
        for label in labels {
            let mut wins = pick_windows(&spans, label, slice.len());
            wins.truncate(WINDOWS_PER_ITEM);
            let vec = match embed_windows(embedder, slice, &wins, stop)? {
                Some(v) => Some(v.vec.clone()),
                None if stop() => return Ok(None),
                None => None,
            };
            items.push(Item {
                win,
                vec,
                segs: segs
                    .iter()
                    .filter(|s| s.speaker == label)
                    .map(|s| (offset + s.start, offset + s.end))
                    .collect(),
            });
        }
    }
    Ok(Some(Diarized { items }))
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// A cluster of items while merging; its centroid is wiped when it drops.
struct Cluster {
    items: Vec<usize>,
    centroid: Option<Vec<f32>>,
    seconds: f64,
    wins: BTreeSet<usize>,
    /// Nothing it may join (everything shares a window with it).
    stuck: bool,
}

impl Drop for Cluster {
    fn drop(&mut self) {
        if let Some(c) = self.centroid.as_mut() {
            c.zeroize();
        }
    }
}

impl Diarized {
    /// Number of (window, label) items and how many have a voice.
    pub fn counts(&self) -> (usize, usize) {
        (
            self.items.len(),
            self.items.iter().filter(|i| i.vec.is_some()).count(),
        )
    }

    fn cluster_of(&self, items: Vec<usize>) -> Option<Cluster> {
        let mut vs: Vec<Vec<f32>> = items
            .iter()
            .map(|&i| self.items[i].vec.clone())
            .collect::<Option<_>>()?;
        let centroid = mean_normalized(&vs);
        vs.iter_mut().for_each(|v| v.zeroize());
        Some(Cluster {
            seconds: items.iter().map(|&i| self.items[i].seconds()).sum(),
            wins: items.iter().map(|&i| self.items[i].win).collect(),
            items,
            centroid,
            stuck: false,
        })
    }

    /// Clusters the items and returns the segments plus the number of voiced
    /// clusters. Labels run `1..` in order of first appearance; an item with
    /// no voice follows the nearest voiced item of a cluster not heard in its
    /// window, and keeps a label of its own when there is none. Two labels of one window never
    /// share a cluster. `None` without at least two voices, or when `stop`
    /// fires.
    pub fn assign(
        &self,
        p: Params,
        stop: &dyn Fn() -> bool,
    ) -> Option<(Vec<SpeakerSegment>, usize)> {
        let with: Vec<usize> = (0..self.items.len())
            .filter(|&i| self.items[i].vec.is_some())
            .collect();
        if with.len() < 2 {
            return None;
        }
        // Average-linkage agglomeration (Lance-Williams on cosine); labels of
        // one window are never the same person: -inf survives every average.
        let n = with.len();
        let mut sim = vec![vec![0.0f32; n]; n];
        for i in 0..n {
            for j in 0..n {
                let (a, b) = (&self.items[with[i]], &self.items[with[j]]);
                sim[i][j] = if i != j && a.win == b.win {
                    f32::NEG_INFINITY
                } else {
                    dot(a.vec.as_deref()?, b.vec.as_deref()?)
                };
            }
        }
        let mut members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
        let mut alive = vec![true; n];
        loop {
            if stop() {
                return None;
            }
            let mut best = (p.threshold, usize::MAX, usize::MAX);
            for i in (0..n).filter(|&i| alive[i]) {
                for j in i + 1..n {
                    if alive[j] && sim[i][j] >= best.0 {
                        best = (sim[i][j], i, j);
                    }
                }
            }
            let (_, i, j) = best;
            if i == usize::MAX {
                break;
            }
            let (ni, nj) = (members[i].len() as f32, members[j].len() as f32);
            for k in (0..n).filter(|&k| alive[k] && k != i && k != j) {
                let s = (ni * sim[i][k] + nj * sim[j][k]) / (ni + nj);
                sim[i][k] = s;
                sim[k][i] = s;
            }
            let moved = std::mem::take(&mut members[j]);
            members[i].extend(moved);
            alive[j] = false;
        }
        let mut clusters: Vec<Cluster> = (0..n)
            .filter(|&i| alive[i])
            .map(|i| self.cluster_of(members[i].iter().map(|&m| with[m]).collect()))
            .collect::<Option<_>>()?;
        // Little speech: not a voice of its own; join the nearest cluster it
        // does not share a window with.
        loop {
            if stop() {
                return None;
            }
            let Some(small) = (0..clusters.len())
                .filter(|&c| clusters[c].seconds < p.min_cluster_s && !clusters[c].stuck)
                .min_by(|&a, &b| clusters[a].seconds.total_cmp(&clusters[b].seconds))
            else {
                break;
            };
            let target = (0..clusters.len())
                .filter(|&c| c != small && clusters[c].wins.is_disjoint(&clusters[small].wins))
                .filter_map(|c| {
                    let s = dot(
                        clusters[small].centroid.as_deref()?,
                        clusters[c].centroid.as_deref()?,
                    );
                    Some((s, c))
                })
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, c)| c);
            let Some(target) = target else {
                clusters[small].stuck = true;
                continue;
            };
            let mut gone = clusters.remove(small);
            let target = if target > small { target - 1 } else { target };
            let mut all = std::mem::take(&mut clusters[target].items);
            all.extend(std::mem::take(&mut gone.items));
            clusters[target] = self.cluster_of(all)?;
        }
        let voiced = clusters.len();
        let cluster_wins: Vec<BTreeSet<usize>> = clusters.iter().map(|c| c.wins.clone()).collect();
        let mut owner = vec![usize::MAX; self.items.len()];
        for (c, cl) in clusters.iter().enumerate() {
            for &i in &cl.items {
                owner[i] = c;
            }
        }
        drop(clusters);
        // No voice: the nearest voiced item in time whose cluster is absent
        // from this item's window (else it would be someone speaking at the
        // same time), otherwise a label of its own.
        let mut extra = voiced;
        for i in 0..self.items.len() {
            if owner[i] != usize::MAX {
                continue;
            }
            let t = self.items[i].start();
            owner[i] = with
                .iter()
                .filter(|&&a| !cluster_wins[owner[a]].contains(&self.items[i].win))
                .min_by(|&&a, &&b| {
                    (self.items[a].start() - t)
                        .abs()
                        .total_cmp(&(self.items[b].start() - t).abs())
                })
                .map(|&a| owner[a])
                .unwrap_or_else(|| {
                    extra += 1;
                    extra - 1
                });
        }
        // Label in order of first appearance.
        let mut first = vec![f64::INFINITY; extra];
        for (i, item) in self.items.iter().enumerate() {
            first[owner[i]] = first[owner[i]].min(item.start());
        }
        let mut order: Vec<usize> = (0..extra).collect();
        order.sort_by(|&a, &b| first[a].total_cmp(&first[b]));
        let mut label = vec![0u32; extra];
        for (rank, c) in order.into_iter().enumerate() {
            label[c] = rank as u32 + 1;
        }
        let mut out: Vec<SpeakerSegment> = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            for &(start, end) in &item.segs {
                out.push(SpeakerSegment {
                    start,
                    end,
                    speaker: label[owner[i]],
                });
            }
        }
        out.sort_by(|a, b| a.start.total_cmp(&b.start));
        Some((out, voiced))
    }
}

/// [`run`] without the [`ENABLED`] gate, with explicit parameters.
pub fn run_with(
    pcm: &[f32],
    engines: &dyn SpeechEngines,
    embedder: &mut dyn VoiceEmbed,
    segs: &[SpeakerSegment],
    stop: &dyn Fn() -> bool,
    params: Params,
) -> Result<Option<Vec<SpeakerSegment>>, String> {
    let mut distinct: Vec<u32> = segs.iter().map(|s| s.speaker).collect();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() < SATURATED_AT {
        return Ok(None);
    }
    let Some(d) = analyse(pcm, engines, embedder, stop)? else {
        return Ok(None);
    };
    Ok(d.assign(params, stop)
        .filter(|(_, n)| *n > SATURATED_AT)
        .map(|(s, _)| s))
}

/// Uncapped segments for `pcm` (16 kHz mono), or `None` to keep `segs`
/// (disabled, not saturated, fewer than 9 clusters result, or stopped).
/// `stop` is asked between windows.
pub fn run(
    pcm: &[f32],
    engines: &dyn SpeechEngines,
    embedder: &mut dyn VoiceEmbed,
    segs: &[SpeakerSegment],
    stop: &dyn Fn() -> bool,
) -> Result<Option<Vec<SpeakerSegment>>, String> {
    if !ENABLED {
        return Ok(None);
    }
    run_with(pcm, engines, embedder, segs, stop, PARAMS)
}
