// SPDX-License-Identifier: Apache-2.0
//! `ghi transcribe`: a `ghi.transcript/1` document, or `ghi.event/1` NDJSON with `--stream`.

use std::path::Path;

use ghi_speech::AsrResult;

use crate::audio::{self, Audio};
use crate::contract::{
    EVENT, Engine, ErrorDoc, Event, EventType, EventWord, Lang, LangMode, Pass, Perf, Segment,
    TRANSCRIPT, Transcript, Word,
};
use crate::engine::{self, EngineArgs};

pub struct Args<'a> {
    pub audio: &'a Path,
    pub lang: LangMode,
    pub pass: Pass,
    pub stream: bool,
    pub realtime: bool,
    pub engine: &'a EngineArgs,
}

pub fn run(args: &Args) -> Result<(), ErrorDoc> {
    crate::check_input_file(args.audio)?;
    let audio = audio::read_wav(args.audio)?;
    run_with_audio(args, &audio)
}

#[cfg(not(feature = "nemo"))]
fn run_with_audio(_args: &Args, _audio: &Audio) -> Result<(), ErrorDoc> {
    Err(engine::unavailable("transcribe"))
}

#[cfg(feature = "nemo")]
fn run_with_audio(args: &Args, audio: &Audio) -> Result<(), ErrorDoc> {
    use std::io::Write;
    use std::time::Instant;

    use ghi_speech::nemo::AsrOptions;

    use crate::contract::ErrorCode;

    fn to_line(ev: &Event) -> String {
        serde_json::to_string(ev).expect("event serializes")
    }

    let started = Instant::now();
    let (asr, engine_info) = engine::load_asr(args.engine, args.pass)?;
    let options = AsrOptions {
        language: language_code(args.lang).map(str::to_owned),
    };
    let mut builder = TranscriptBuilder::default();
    if args.engine.offline && !args.stream {
        let mut r = asr
            .recognize(&audio.samples, audio.sample_rate, &options)
            .map_err(engine::speech_error)?;
        r.is_final = true;
        builder.add(&r);
    } else {
        let mut stream = asr.stream(&options).map_err(engine::speech_error)?;
        if args.stream {
            let mut out = std::io::stdout().lock();
            let mut events = EventWriter::default();
            let mut write_err = None;
            crate::cmd::feed_asr(&mut stream, audio, args.realtime, |r, wall| {
                if let Some(ev) = events.event(&r, wall)
                    && let Err(e) = writeln!(out, "{}", to_line(&ev))
                {
                    // Reader went away (e.g. `| head`): stop instead of feeding on.
                    write_err = Some(e);
                    return false;
                }
                true
            })
            .map_err(engine::speech_error)?;
            let end = events.end(started.elapsed().as_secs_f64(), audio.duration_s());
            return write_err
                .map_or_else(|| writeln!(out, "{}", to_line(&end)), Err)
                .map_err(|e| ErrorDoc::new(ErrorCode::Internal, e.to_string()));
        }
        crate::cmd::feed_asr(&mut stream, audio, false, |r, _| {
            builder.add(&r);
            true
        })
        .map_err(engine::speech_error)?;
    }
    let doc = builder.finish(args, audio, engine_info, started.elapsed().as_secs_f64());
    crate::emit(&doc)
}

/// `--lang` to the engine's BCP-47 prompt; `None` = automatic detection.
pub fn language_code(lang: LangMode) -> Option<&'static str> {
    match lang {
        LangMode::Auto => None,
        LangMode::Vi => Some("vi-VN"),
        LangMode::En => Some("en-US"),
    }
}

/// Engine language tag (`vi-VN`, `en-US`, ...) to the contract's `vi`/`en`.
pub fn lang_of(r: &AsrResult) -> Option<Lang> {
    let code = r.languages.first()?.to_ascii_lowercase();
    if code.starts_with("vi") {
        Some(Lang::Vi)
    } else if code.starts_with("en") {
        Some(Lang::En)
    } else {
        None
    }
}

/// Collects final results into transcript segments.
#[derive(Default)]
pub struct TranscriptBuilder {
    segments: Vec<Segment>,
    last_end: f64,
}

impl TranscriptBuilder {
    pub fn add(&mut self, r: &AsrResult) {
        if !r.is_final {
            return;
        }
        let (start, end) = span(r, self.last_end);
        self.last_end = end;
        let text = r.text.trim();
        if text.is_empty() {
            return;
        }
        self.segments.push(Segment {
            id: self.segments.len() as u32,
            start,
            end,
            text: text.to_owned(),
            lang: lang_of(r),
            speaker: None,
            words: Some(
                r.words
                    .iter()
                    .map(|w| Word {
                        start: w.start,
                        end: w.end,
                        text: w.text.clone(),
                    })
                    .collect(),
            ),
        });
    }

    pub fn finish(self, args: &Args, audio: &Audio, engine: Engine, wall_s: f64) -> Transcript {
        let duration_s = audio.duration_s();
        Transcript {
            schema: TRANSCRIPT.to_owned(),
            audio: audio::display_name(args.audio),
            duration_s,
            pass: args.pass,
            lang: args.lang,
            engine,
            segments: self.segments,
            perf: Perf {
                wall_s,
                rtf: (duration_s > 0.0).then(|| wall_s / duration_s),
                peak_rss_mb: engine::peak_rss_mb(),
            },
        }
    }
}

/// Audio span of a result: its words, else from the previous final to the
/// audio consumed so far.
fn span(r: &AsrResult, last_end: f64) -> (f64, f64) {
    match (r.words.first(), r.words.last()) {
        (Some(first), Some(last)) => (first.start, last.end),
        _ => (last_end, r.audio_processed.max(last_end)),
    }
}

/// Turns results into `ghi.event/1` lines.
///
/// Partials carry no word times, so caption lag is measured on the words of
/// each final: a word counts as shown at the first partial whose token at the
/// same position equals it (ignoring case and punctuation), else at the final.
#[derive(Default)]
pub struct EventWriter {
    seq: u64,
    last_final_end: f64,
    /// Partials of the current utterance: (wall_s, normalized tokens).
    partials: Vec<(f64, Vec<String>)>,
}

impl EventWriter {
    /// `None` for results with no text (nothing to show as a caption).
    pub fn event(&mut self, r: &AsrResult, wall_s: f64) -> Option<Event> {
        let (start, end) = span(r, self.last_final_end);
        let text = r.text.trim();
        let (kind, words) = if r.is_final {
            self.last_final_end = end;
            let words = r
                .words
                .iter()
                .enumerate()
                .map(|(i, w)| EventWord {
                    start: w.start,
                    end: w.end,
                    text: w.text.clone(),
                    shown_s: self.first_shown(i, &w.text).unwrap_or(wall_s),
                })
                .collect();
            self.partials.clear();
            (EventType::Final, Some(words))
        } else {
            self.partials
                .push((wall_s, text.split_whitespace().map(token).collect()));
            (EventType::Partial, None)
        };
        if text.is_empty() {
            return None;
        }
        Some(self.next(kind, wall_s, start, end, text, lang_of(r), words))
    }

    pub fn end(&mut self, wall_s: f64, duration_s: f64) -> Event {
        self.next(EventType::End, wall_s, 0.0, duration_s, "", None, None)
    }

    fn first_shown(&self, i: usize, word: &str) -> Option<f64> {
        let word = token(word);
        self.partials
            .iter()
            .find(|(_, tokens)| tokens.get(i) == Some(&word))
            .map(|(wall, _)| *wall)
    }

    #[allow(clippy::too_many_arguments)]
    fn next(
        &mut self,
        kind: EventType,
        wall_s: f64,
        audio_start: f64,
        audio_end: f64,
        text: &str,
        lang: Option<Lang>,
        words: Option<Vec<EventWord>>,
    ) -> Event {
        let seq = self.seq;
        self.seq += 1;
        Event {
            schema: EVENT.to_owned(),
            kind,
            seq,
            wall_s,
            audio_start,
            audio_end,
            text: text.to_owned(),
            lang,
            speaker: None,
            words,
        }
    }
}

/// Lowercase, without leading/trailing punctuation, for matching partial tokens.
fn token(t: &str) -> String {
    t.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::fake::{result, word};

    #[test]
    fn finals_become_segments_with_words_and_language() {
        let mut b = TranscriptBuilder::default();
        b.add(&result(false, "vi-VN", vec![word("Mình", 0.4, 0.6)]));
        b.add(&result(
            true,
            "vi-VN",
            vec![word("Mình", 0.4, 0.6), word("chốt", 0.7, 0.9)],
        ));
        b.add(&result(true, "en-US", vec![word("okay", 1.5, 1.8)]));
        b.add(&result(true, "vi-VN", vec![]));
        let s = &b.segments;
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].id, s[0].start, s[0].end), (0, 0.4, 0.9));
        assert_eq!(s[0].text, "Mình chốt");
        assert_eq!(s[0].lang, Some(Lang::Vi));
        assert_eq!(s[1].lang, Some(Lang::En));
        assert_eq!(s[0].words.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn events_carry_sequence_and_spans() {
        let mut w = EventWriter::default();
        let p = w.event(&result(false, "vi-VN", vec![]), 0.9);
        assert!(p.is_none(), "empty partial");
        let mut partial = result(false, "vi-VN", vec![]);
        partial.text = "xin".into();
        partial.audio_processed = 0.5;
        let p = w.event(&partial, 1.0).unwrap();
        let f = w
            .event(
                &result(
                    true,
                    "vi-VN",
                    vec![word("xin", 0.1, 0.3), word("chào", 0.4, 0.7)],
                ),
                1.6,
            )
            .unwrap();
        let e = w.end(2.0, 2.5);
        assert_eq!(
            (p.kind, p.seq, p.audio_end, p.words.is_none()),
            (EventType::Partial, 0, 0.5, true)
        );
        assert_eq!(
            (f.kind, f.seq, f.audio_end, f.wall_s),
            (EventType::Final, 1, 0.7, 1.6)
        );
        assert_eq!((e.kind, e.seq, e.audio_end), (EventType::End, 2, 2.5));
    }

    #[test]
    fn words_are_shown_at_first_matching_partial() {
        let mut w = EventWriter::default();
        for (wall, text) in [(1.0, "Văn"), (1.5, "Văn hóa và b"), (2.0, "Văn hóa và bộ")] {
            let mut r = result(false, "vi-VN", vec![]);
            r.text = text.into();
            w.event(&r, wall);
        }
        let f = w
            .event(
                &result(
                    true,
                    "vi-VN",
                    vec![
                        word("Văn", 0.2, 0.5),
                        word("hóa", 0.6, 0.9),
                        word("và", 1.0, 1.1),
                        word("bộ", 1.2, 1.4),
                        word("lạc,", 1.5, 1.8),
                    ],
                ),
                3.0,
            )
            .unwrap();
        let shown: Vec<f64> = f.words.unwrap().iter().map(|w| w.shown_s).collect();
        assert_eq!(shown, vec![1.0, 1.5, 1.5, 2.0, 3.0]);
        // The next utterance starts with no partials.
        let f2 = w
            .event(&result(true, "vi-VN", vec![word("tiếp", 4.0, 4.2)]), 5.0)
            .unwrap();
        assert_eq!(f2.words.unwrap()[0].shown_s, 5.0);
    }

    #[test]
    fn languages_map_to_contract() {
        assert_eq!(language_code(LangMode::Vi), Some("vi-VN"));
        assert_eq!(language_code(LangMode::Auto), None);
        assert_eq!(lang_of(&result(true, "fr-FR", vec![])), None);
    }
}
