// SPDX-License-Identifier: Apache-2.0
//! The model-free half of the Whisper engine: grouping VAD speech regions into
//! decodable windows, turning tokens into timed words, and the hallucination
//! guard. Pure functions, so they are tested without a model.

use crate::Word;

/// Longest window handed to one `whisper_full` call (its context is 30 s).
pub const MAX_GROUP_S: f64 = 29.0;
/// Shortest window decoded; shorter audio is padded with silence by the caller.
pub const MIN_GROUP_S: f64 = 1.0;
/// Speech regions closer than this are one span, silence between them kept.
/// Whisper reads better with the natural pauses of a conversation than with
/// sentences cut together (FLEURS WER 9% became 11-14% at 1.5 s), so only long
/// silences, where it invents text, are cut.
pub const JOIN_GAP_S: f64 = 6.0;
/// Silence put between spans that are cut together into one window: a pause's worth.
pub const SEP_S: f64 = 1.0;

/// A stretch of speech found by the VAD, seconds from the start of the audio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub start: f64,
    pub end: f64,
}

/// A stretch of the source audio copied into a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Piece {
    /// Where it sits in the source audio.
    pub src: Region,
    /// Where it starts in the window.
    pub at: f64,
}

/// One decodable window: pieces of the source, cut from its speech and joined
/// with [`SEP_S`] of silence, so long pauses are never decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub pieces: Vec<Piece>,
    /// Length in seconds (without any padding the caller adds at the end).
    pub len: f64,
    /// The VAD's regions that fall in the window, on the window's timeline.
    pub speech: Vec<Region>,
}

impl Window {
    fn push(&mut self, src: Region, members: &[Region]) {
        let at = if self.pieces.is_empty() {
            0.0
        } else {
            self.len + SEP_S
        };
        self.pieces.push(Piece { src, at });
        self.len = at + (src.end - src.start);
        for m in members {
            let (a, b) = (m.start.max(src.start), m.end.min(src.end));
            if b > a {
                self.speech.push(Region {
                    start: at + a - src.start,
                    end: at + b - src.start,
                });
            }
        }
    }

    /// A window time as a source time. Inside a separator a start goes to the
    /// next piece's start and an end to the previous piece's end.
    pub fn to_source(&self, t: f64, is_start: bool) -> f64 {
        for (i, p) in self.pieces.iter().enumerate() {
            let len = p.src.end - p.src.start;
            if t < p.at + len {
                if t >= p.at {
                    return p.src.start + (t - p.at);
                }
                // In the separator before this piece.
                return if is_start {
                    p.src.start
                } else {
                    self.pieces[i.saturating_sub(1)].src.end
                };
            }
        }
        self.pieces.last().map_or(t, |p| p.src.end)
    }
}

/// A span longer than [`MAX_GROUP_S`] cut at the gaps between its regions
/// (never mid-word); a single region over the limit is cut hard.
fn split_span(span: Region, members: Vec<Region>) -> Vec<(Region, Vec<Region>)> {
    if span.end - span.start <= MAX_GROUP_S {
        return vec![(span, members)];
    }
    let mut out: Vec<(Region, Vec<Region>)> = Vec::new();
    let mut cur: Option<(Region, Vec<Region>)> = None;
    for m in members {
        // A region too long for any window: hard cuts.
        if m.end - m.start > MAX_GROUP_S {
            out.extend(cur.take());
            let mut a = m.start;
            while a < m.end {
                let b = (a + MAX_GROUP_S).min(m.end);
                out.push((Region { start: a, end: b }, vec![m]));
                a = b;
            }
            continue;
        }
        cur = match cur.take() {
            Some((s, mut v)) if m.end - s.start <= MAX_GROUP_S => {
                v.push(m);
                Some((
                    Region {
                        start: s.start,
                        end: m.end,
                    },
                    v,
                ))
            }
            Some(done) => {
                out.push(done);
                Some((m, vec![m]))
            }
            None => Some((m, vec![m])),
        };
    }
    out.extend(cur);
    out
}

/// Packs regions, in order, into windows of at most [`MAX_GROUP_S`]: regions
/// within [`JOIN_GAP_S`] of each other form a span (their silence kept), and
/// spans are joined into a window with [`SEP_S`] between them. A span longer
/// than the limit is cut into pieces.
pub fn plan_windows(regions: &[Region]) -> Vec<Window> {
    // Spans with the regions inside them.
    let mut spans: Vec<(Region, Vec<Region>)> = Vec::new();
    for r in regions.iter().filter(|r| r.end > r.start) {
        match spans.last_mut() {
            Some((s, members)) if r.start - s.end <= JOIN_GAP_S => {
                s.end = s.end.max(r.end);
                members.push(*r);
            }
            _ => spans.push((*r, vec![*r])),
        }
    }
    let mut windows: Vec<Window> = Vec::new();
    let mut cur = Window {
        pieces: Vec::new(),
        len: 0.0,
        speech: Vec::new(),
    };
    for (span, members) in spans.into_iter().flat_map(|(s, m)| split_span(s, m)) {
        let len = span.end - span.start;
        if !cur.pieces.is_empty() && cur.len + SEP_S + len > MAX_GROUP_S {
            windows.push(std::mem::replace(
                &mut cur,
                Window {
                    pieces: Vec::new(),
                    len: 0.0,
                    speech: Vec::new(),
                },
            ));
        }
        cur.push(span, &members);
    }
    if !cur.pieces.is_empty() {
        windows.push(cur);
    }
    windows
}

/// One decoded token as the engine reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct RawTok {
    /// Raw bytes: a Vietnamese letter can be split across byte-fallback tokens.
    pub bytes: Vec<u8>,
    pub p: f32,
    /// DTW time, seconds from the window start; `None` when unknown.
    pub t: Option<f64>,
}

/// One decoded segment: times are seconds from the window start; `toks` are
/// the text tokens only.
#[derive(Debug, Clone, PartialEq)]
pub struct RawSeg {
    pub t0: f64,
    pub t1: f64,
    pub no_speech: f32,
    pub text: String,
    pub toks: Vec<RawTok>,
}

/// Words of a segment on the stream timeline (`offset` is the window's start;
/// times are clipped to the window, `win_len` long).
///
/// A word starts at the DTW time of its first token and ends where the next
/// begins (the segment's end for the last one). Tokens that begin with a space
/// start a word; the rest (punctuation, byte pieces) join the previous one.
pub fn words(seg: &RawSeg, offset: f64, win_len: f64) -> Vec<Word> {
    struct Acc {
        bytes: Vec<u8>,
        start: Option<f64>,
        p: Vec<f32>,
    }
    let mut accs: Vec<Acc> = Vec::new();
    for tok in &seg.toks {
        let new_word = accs.is_empty() || tok.bytes.first() == Some(&b' ');
        if new_word {
            accs.push(Acc {
                bytes: Vec::new(),
                start: None,
                p: Vec::new(),
            });
        }
        let a = accs.last_mut().expect("pushed above");
        a.bytes.extend_from_slice(&tok.bytes);
        a.p.push(tok.p);
        if a.start.is_none() {
            a.start = tok.t;
        }
    }
    let texts: Vec<String> = accs
        .iter()
        .map(|a| String::from_utf8_lossy(&a.bytes).trim().to_string())
        .collect();
    let seg_end = seg.t1.min(win_len).max(seg.t0);
    let mut out: Vec<Word> = Vec::with_capacity(accs.len());
    let mut prev_end = seg.t0.clamp(0.0, win_len);
    // Starts: DTW where known, else carried from the previous word.
    let starts: Vec<f64> = accs
        .iter()
        .map(|a| {
            let s = a.start.unwrap_or(prev_end).clamp(prev_end, win_len);
            prev_end = s;
            s
        })
        .collect();
    for (i, (a, text)) in accs.iter().zip(&texts).enumerate() {
        if text.is_empty() {
            continue;
        }
        let start = starts[i];
        let end = starts
            .get(i + 1)
            .copied()
            .unwrap_or(seg_end)
            .max(start)
            .min(win_len);
        out.push(Word {
            text: text.clone(),
            start: offset + start,
            end: offset + end,
            confidence: a.p.iter().sum::<f32>() / a.p.len().max(1) as f32,
            speaker: None,
        });
    }
    out
}

/// A segment is dropped when it is likely invented: the model itself says
/// there is no speech and is unsure; it is mostly one phrase on repeat and
/// either the model is unsure of it or the phrase repeats 8 or more times
/// (people do say "vâng vâng vâng vâng vâng"); or it sits (mostly) outside the
/// VAD's speech regions; or it is a video-channel sign-off Whisper learned from
/// YouTube captions ([`OUTRO`]). `regions` and the segment's `start..end` are
/// on the same timeline.
pub fn keep(seg: &RawSeg, words: &[Word], start: f64, end: f64, regions: &[Region]) -> bool {
    if seg.text.trim().is_empty() || words.is_empty() {
        return false;
    }
    // OpenAI's rule: no-speech probability high and average log-probability low.
    let avg_logprob = seg
        .toks
        .iter()
        .map(|t| f64::from(t.p.max(1e-6)).ln())
        .sum::<f64>()
        / seg.toks.len().max(1) as f64;
    if seg.no_speech > 0.6 && avg_logprob < -1.0 {
        return false;
    }
    let tokens: Vec<String> = words.iter().map(|w| normalize(&w.text)).collect();
    if is_outro(&tokens) {
        return false;
    }
    let (fraction, reps) = longest_loop(&tokens);
    if fraction >= 0.5 && (reps >= 8 || avg_logprob < -0.8 || seg.no_speech > 0.4) {
        return false;
    }
    let span = end - start;
    if span > 0.0 {
        let covered: f64 = regions
            .iter()
            .map(|r| (end.min(r.end) - start.max(r.start)).max(0.0))
            .sum();
        if covered / span < 0.3 {
            return false;
        }
    }
    true
}

/// Sign-offs of video channels that Whisper writes over real speech it cannot
/// place, with full confidence (no-speech 0, log-probability about -0.05), so
/// the checks above miss them: on VietMed's 8 kHz clips the whole clip came out
/// as "Hãy subscribe cho kênh Ghiền Mì Gõ…", with or without padding or beam
/// search. Phrases are matched on normalized words, in order.
const OUTRO: &[&[&str]] = &[
    &["subscribe", "cho", "kênh"],
    &["đăng", "ký", "kênh"],
    &["ủng", "hộ", "cho", "kênh"],
    &["ủng", "hộ", "kênh"],
    &["ghiền", "mì", "gõ"],
    &["không", "bỏ", "lỡ", "những", "video"],
    &["thanks", "for", "watching"],
    &["thank", "you", "for", "watching"],
    &["like", "and", "subscribe"],
];

fn is_outro(tokens: &[String]) -> bool {
    OUTRO.iter().any(|phrase| {
        tokens
            .windows(phrase.len())
            .any(|w| w.iter().zip(*phrase).all(|(a, b)| a == b))
    })
}

/// Lowercase, letters and digits only: "Vâng," and "vâng" are the same token.
fn normalize(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The largest run of one phrase (1 to 4 tokens) repeated back to back, as
/// (fraction of the tokens it covers, repeats). Only runs long enough to be a
/// loop count: a word 5 times, a pair 4, a triple or quad 3.
fn longest_loop(tokens: &[String]) -> (f64, usize) {
    if tokens.is_empty() {
        return (0.0, 0);
    }
    let (mut best, mut best_reps) = (0usize, 0usize);
    for n in 1..=4usize {
        let min_reps = match n {
            1 => 5,
            2 => 4,
            _ => 3,
        };
        let mut i = 0;
        while i + n <= tokens.len() {
            let mut reps = 1;
            while i + (reps + 1) * n <= tokens.len()
                && tokens[i..i + n] == tokens[i + reps * n..i + (reps + 1) * n]
            {
                reps += 1;
            }
            if reps >= min_reps {
                if reps * n > best {
                    (best, best_reps) = (reps * n, reps);
                }
                i += reps * n;
            } else {
                i += 1;
            }
        }
    }
    (best as f64 / tokens.len() as f64, best_reps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: f64, end: f64) -> Region {
        Region { start, end }
    }

    fn tok(s: &str, p: f32, t: Option<f64>) -> RawTok {
        RawTok {
            bytes: s.as_bytes().to_vec(),
            p,
            t,
        }
    }

    fn seg(toks: Vec<RawTok>, t0: f64, t1: f64, no_speech: f32) -> RawSeg {
        let text = toks
            .iter()
            .map(|t| String::from_utf8_lossy(&t.bytes).into_owned())
            .collect::<String>();
        RawSeg {
            t0,
            t1,
            no_speech,
            text,
            toks,
        }
    }

    #[test]
    fn close_regions_are_one_span_and_far_ones_are_cut_together() {
        // 1..5 and 6..9 are within the join gap (one span); 30..34 is far, so
        // its silence is dropped: both go in one window with a pause between.
        let w = plan_windows(&[r(1.0, 5.0), r(6.0, 9.0), r(30.0, 34.0)]);
        assert_eq!(w.len(), 1);
        let w = &w[0];
        assert_eq!(w.pieces.len(), 2);
        assert_eq!(w.pieces[0].src, r(1.0, 9.0));
        assert_eq!((w.pieces[1].at, w.pieces[1].src), (9.0, r(30.0, 34.0)));
        assert_eq!(w.len, 13.0);
        assert_eq!(w.speech, vec![r(0.0, 4.0), r(5.0, 8.0), r(9.0, 13.0)]);
        assert!(plan_windows(&[]).is_empty());
    }

    #[test]
    fn windows_fill_up_to_the_limit() {
        // 12 s + 1 s + 12 s fits in 29 s; a third 12 s span does not.
        let w = plan_windows(&[r(0.0, 12.0), r(20.0, 32.0), r(40.0, 52.0)]);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].pieces.len(), 2);
        assert_eq!(w[1].pieces.len(), 1);
        assert!(w.iter().all(|x| x.len <= MAX_GROUP_S));
    }

    #[test]
    fn long_runs_of_speech_are_cut_at_pauses_not_mid_word() {
        // Back-to-back 10 s regions 0.3 s apart: one long span, cut between
        // regions (two per window), never inside one.
        let regions: Vec<Region> = (0..6)
            .map(|i| r(f64::from(i) * 10.3, f64::from(i) * 10.3 + 10.0))
            .collect();
        let w = plan_windows(&regions);
        assert!(w.len() >= 3, "{w:?}");
        for win in &w {
            assert!(win.len <= MAX_GROUP_S);
            for p in &win.pieces {
                assert!(
                    regions.iter().any(|x| x.start == p.src.start)
                        && regions.iter().any(|x| x.end == p.src.end),
                    "a piece starts or ends inside a region: {p:?}"
                );
            }
        }
    }

    #[test]
    fn long_spans_are_cut() {
        let w = plan_windows(&[r(0.0, 70.0)]);
        let src: Vec<_> = w.iter().map(|x| x.pieces[0].src).collect();
        assert_eq!(src, vec![r(0.0, 29.0), r(29.0, 58.0), r(58.0, 70.0)]);
    }

    #[test]
    fn window_times_map_back_to_the_source() {
        let w = &plan_windows(&[r(1.0, 5.0), r(30.0, 34.0)])[0];
        // Pieces: src 1..5 at 0; src 30..34 at 5.
        assert_eq!(w.to_source(2.0, true), 3.0);
        assert_eq!(w.to_source(6.0, true), 31.0);
        // In the separator (4.0..5.0): a start goes forward, an end back.
        assert_eq!(w.to_source(4.5, true), 30.0);
        assert_eq!(w.to_source(4.5, false), 5.0);
        // Past the end.
        assert_eq!(w.to_source(99.0, false), 34.0);
    }

    #[test]
    fn words_join_byte_pieces_and_follow_dtw_times() {
        // "Xin chào," then "bạn" whose "ạ" (E1 BA A1) arrives as byte pieces.
        let toks = vec![
            tok(" Xin", 0.9, Some(0.50)),
            tok(" ch", 0.8, Some(0.80)),
            tok("ào", 0.8, Some(1.00)),
            tok(",", 0.7, Some(1.20)),
            tok(" b", 0.6, Some(1.40)),
            RawTok {
                bytes: vec![0xE1, 0xBA],
                p: 0.5,
                t: Some(1.55),
            },
            RawTok {
                bytes: vec![0xA1, b'n'],
                p: 0.5,
                t: Some(1.70),
            },
        ];
        let s = seg(toks, 0.4, 2.0, 0.0);
        let w = words(&s, 10.0, 30.0);
        let texts: Vec<_> = w.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Xin", "chào,", "bạn"]);
        assert_eq!((w[0].start, w[0].end), (10.5, 10.8));
        assert_eq!((w[1].start, w[1].end), (10.8, 11.4));
        assert_eq!((w[2].start, w[2].end), (11.4, 12.0));
        assert!((w[1].confidence - (0.8 + 0.8 + 0.7) / 3.0).abs() < 1e-6);
    }

    #[test]
    fn words_without_dtw_times_stay_ordered_and_inside_the_window() {
        let toks = vec![
            tok(" một", 0.9, None),
            tok(" hai", 0.9, Some(0.9)),
            tok(" ba", 0.9, Some(0.5)), // out of order: held at the previous start
            tok(" bốn", 0.9, Some(99.0)), // past the window: clipped
        ];
        let s = seg(toks, 0.2, 5.0, 0.0);
        let w = words(&s, 0.0, 3.0);
        assert_eq!(w.len(), 4);
        let mut prev = 0.0;
        for x in &w {
            assert!(x.start >= prev && x.end >= x.start && x.end <= 3.0, "{x:?}");
            prev = x.start;
        }
        assert_eq!(w[0].start, 0.2, "no time: carried from the segment start");
        assert_eq!(w[3].end, 3.0);
    }

    fn ws(s: &str) -> Vec<Word> {
        s.split_whitespace()
            .enumerate()
            .map(|(i, t)| Word {
                text: t.into(),
                start: i as f64,
                end: i as f64 + 1.0,
                confidence: 0.9,
                speaker: None,
            })
            .collect()
    }

    fn keep_text(text: &str, no_speech: f32, p: f32, regions: &[Region]) -> bool {
        let toks = text
            .split_whitespace()
            .map(|t| tok(&format!(" {t}"), p, None))
            .collect();
        let s = seg(toks, 0.0, 10.0, no_speech);
        keep(&s, &ws(text), 0.0, 10.0, regions)
    }

    #[test]
    fn guard_keeps_normal_speech() {
        assert!(keep_text("xin chào mọi người", 0.01, 0.9, &[r(0.0, 10.0)]));
        // "Vâng vâng vâng" is speech, not a loop.
        assert!(keep_text(
            "vâng vâng vâng được rồi",
            0.0,
            0.9,
            &[r(0.0, 10.0)]
        ));
    }

    #[test]
    fn guard_drops_no_speech_loops_and_text_outside_speech() {
        // The model doubts there is speech and is unsure of its words.
        assert!(!keep_text("cảm ơn các bạn", 0.8, 0.2, &[r(0.0, 10.0)]));
        // Sure of its words: kept even with a high no-speech probability.
        assert!(keep_text("cảm ơn các bạn", 0.8, 0.9, &[r(0.0, 10.0)]));
        // A phrase on repeat fills the segment, and the model is unsure of it
        // or it repeats 8 or more times.
        assert!(!keep_text(
            "đi đi đi đi đi đi đi đi",
            0.0,
            0.9,
            &[r(0.0, 10.0)]
        ));
        assert!(!keep_text(
            "hẹn gặp lại hẹn gặp lại hẹn gặp lại hẹn gặp lại",
            0.0,
            0.3,
            &[r(0.0, 10.0)]
        ));
        assert!(!keep_text("đi đi đi đi đi đi", 0.5, 0.9, &[r(0.0, 10.0)]));
        // Decoded over a stretch with no speech in it.
        assert!(!keep_text("xin chào mọi người", 0.0, 0.9, &[r(8.0, 10.0)]));
        // Empty.
        assert!(!keep(
            &seg(vec![], 0.0, 1.0, 0.0),
            &[],
            0.0,
            1.0,
            &[r(0.0, 1.0)]
        ));
    }

    #[test]
    fn guard_drops_channel_sign_offs() {
        let all = [r(0.0, 10.0)];
        // What Whisper wrote over VietMed clips, fully confident.
        assert!(!keep_text(
            "Hãy subscribe cho kênh Ghiền Mì Gõ Để không bỏ lỡ những video hấp dẫn",
            0.0,
            0.95,
            &all
        ));
        assert!(!keep_text(
            "Các bạn hãy đăng ký kênh để ủng hộ kênh của mình nhé.",
            0.0,
            0.95,
            &all
        ));
        assert!(!keep_text("Thanks for watching!", 0.0, 0.95, &all));
        // The words alone, or out of order, are ordinary speech.
        assert!(keep_text(
            "kênh bán hàng này cần đăng ký thêm",
            0.0,
            0.9,
            &all
        ));
        assert!(keep_text(
            "we should subscribe to that service",
            0.0,
            0.9,
            &all
        ));
        assert!(keep_text(
            "cảm ơn các bạn đã theo dõi buổi họp",
            0.0,
            0.9,
            &all
        ));
    }

    #[test]
    fn loop_inside_long_speech_is_not_enough_to_drop() {
        let text = "tôi nghĩ rằng chúng ta nên làm việc này vào tuần sau \
                    ha ha ha ha ha rồi sau đó mình sẽ báo cáo lại cho mọi người";
        assert!(keep_text(text, 0.0, 0.9, &[r(0.0, 10.0)]));
    }
}
