// SPDX-License-Identifier: Apache-2.0
//! Reads a stretch of recorded audio with a streaming recognizer. After a line
//! the engine asks for ([`SpeechEngines::reset_after`]: NeMo, a Vietnamese
//! one) the stream starts over **at the end of that line's last word**: the
//! old stream, which has heard the next words already, is dropped and the
//! audio since is read again (live capture can only reopen where it is).
//!
//! One Nemotron stream over a minute of Vietnamese drifts. Clips read back to
//! back (3 min, 60 s chunks as the final pass cuts them; 2026-10-10): ViMedCSS
//! 33.0% -> 14.2%, VietMed 18.9% -> 15.9%, FLEURS-vi 8.4% -> 8.6%, English
//! untouched. Reopening where reading got to instead (as live does) lost the
//! next sentence's start (FLEURS-vi 14.3%); cutting long utterances as live
//! does cost FLEURS-vi 2 points and VietMed 2.4.
//!
//! [`SpeechEngines::reset_after`]: crate::engines::SpeechEngines::reset_after

use ghi_audio::SAMPLE_RATE;
use ghi_speech::AsrResult;

use crate::engines::BoxAsr;

const RATE: f64 = SAMPLE_RATE as f64;

/// Audio pushed per step.
const STEP: usize = SAMPLE_RATE as usize / 4;

/// The final results for `pcm`, times on its timeline (seconds from its first
/// sample). `open` starts a stream; `reset_after` says whether a final line
/// asks for a fresh one. `None` when `abort` stopped it (an engine that
/// decodes at the end, Whisper, stops mid-way; nothing is returned then).
pub fn read_finals(
    open: &dyn Fn() -> Result<BoxAsr, String>,
    reset_after: &dyn Fn(&str) -> bool,
    pcm: &[f32],
    abort: &(dyn Fn() -> bool + Sync),
) -> Result<Option<Vec<AsrResult>>, String> {
    let mut reader = Reader {
        stream: open()?,
        from: 0,
        pos: 0,
        out: Vec::new(),
    };
    while reader.pos < pcm.len() {
        if abort() {
            return Ok(None);
        }
        let end = (reader.pos + STEP).min(pcm.len());
        reader
            .stream
            .push(&pcm[reader.pos..end], SAMPLE_RATE)
            .map_err(|e| e.to_string())?;
        reader.pos = end;
        if let Some(at) = reader.drain(Some(reset_after))? {
            reader.restart(open()?, at);
        }
    }
    if !reader.finish(abort)? {
        return Ok(None);
    }
    Ok(Some(reader.out))
}

struct Reader {
    stream: BoxAsr,
    /// Sample where the current stream started.
    from: usize,
    /// Samples pushed so far.
    pos: usize,
    out: Vec<AsrResult>,
}

impl Reader {
    /// Takes the stream's results. With `reset_after`, stops at a final that
    /// asks for a fresh stream and returns where that one starts: the end of
    /// the final's last word, if that is past the stream's start (always
    /// progress).
    fn drain(
        &mut self,
        reset_after: Option<&dyn Fn(&str) -> bool>,
    ) -> Result<Option<usize>, String> {
        while let Some(mut r) = self.stream.next_result().map_err(|e| e.to_string())? {
            if !r.is_final || r.text.trim().is_empty() {
                continue;
            }
            let off = self.from as f64 / RATE;
            for w in &mut r.words {
                w.start += off;
                w.end += off;
            }
            r.audio_processed += off;
            let reset = reset_after.is_some_and(|f| f(&r.text));
            let end = r.words.last().map(|w| (w.end * RATE).round() as usize);
            self.out.push(r);
            if reset && let Some(at) = end.filter(|&at| at > self.from && at <= self.pos) {
                return Ok(Some(at));
            }
        }
        Ok(None)
    }

    /// A fresh stream from sample `at` (at most where reading got to).
    fn restart(&mut self, stream: BoxAsr, at: usize) {
        self.stream = stream;
        self.from = at;
        self.pos = at;
    }

    /// Flushes the stream; `false` when `abort` stopped it.
    fn finish(&mut self, abort: &(dyn Fn() -> bool + Sync)) -> Result<bool, String> {
        if !self
            .stream
            .finish_abortable(abort)
            .map_err(|e| e.to_string())?
        {
            return Ok(false);
        }
        self.drain(None)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use ghi_speech::{AsrStream, Word};

    use super::*;

    /// Says each scripted line once its audio is in (a final line also when
    /// finished): `(at_s, text, final)`, times from the stream's start. Its
    /// one word ends 0.5 s before (the pause that ends an utterance).
    struct Scripted {
        script: Vec<(f64, &'static str, bool)>,
        pushed: usize,
        finished: bool,
    }

    impl AsrStream for Scripted {
        fn push(&mut self, pcm: &[f32], _rate: u32) -> ghi_speech::Result<()> {
            self.pushed += pcm.len();
            Ok(())
        }
        fn finish(&mut self) -> ghi_speech::Result<()> {
            self.finished = true;
            Ok(())
        }
        fn next_result(&mut self) -> ghi_speech::Result<Option<AsrResult>> {
            let now = self.pushed as f64 / RATE;
            let i = self
                .script
                .iter()
                .position(|(at, _, fin)| *at <= now || (self.finished && *fin));
            Ok(i.map(|i| {
                let (at, text, is_final) = self.script.remove(i);
                AsrResult {
                    is_final,
                    text: text.into(),
                    words: vec![Word {
                        text: text.into(),
                        start: at - 0.7,
                        end: at - 0.5,
                        confidence: 1.0,
                        speaker: None,
                    }],
                    languages: Vec::new(),
                    audio_processed: now,
                }
            }))
        }
    }

    /// Finals over `seconds` of audio, one script per stream opened, and how
    /// many streams were opened.
    fn run(scripts: Vec<Vec<(f64, &'static str, bool)>>, seconds: f64) -> (Vec<AsrResult>, usize) {
        let scripts = RefCell::new(scripts.into_iter());
        let opened = Cell::new(0);
        let open = || -> Result<BoxAsr, String> {
            opened.set(opened.get() + 1);
            Ok(Box::new(Scripted {
                script: scripts.borrow_mut().next().unwrap_or_default(),
                pushed: 0,
                finished: false,
            }))
        };
        let pcm = vec![0.0f32; (seconds * RATE) as usize];
        let out = read_finals(&open, &ghi_text::has_diacritics, &pcm, &|| false)
            .unwrap()
            .unwrap();
        (out, opened.get())
    }

    #[test]
    fn a_vietnamese_line_restarts_the_stream_at_its_last_word() {
        let (out, opened) = run(
            vec![
                // The first stream has heard the next words when the
                // Vietnamese final comes (at 1.0 s, last word ending 0.5 s).
                vec![(1.0, "xin chào", true), (1.5, "lost", true)],
                vec![(1.0, "hello there", true), (2.0, "again", true)],
            ],
            4.0,
        );
        let texts: Vec<&str> = out.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["xin chào", "hello there", "again"]);
        assert_eq!(opened, 2);
        // The second stream started at the end of "xin chào" (0.5 s), not
        // where reading got to (1.0 s); its words are on the stretch's timeline.
        assert!(
            (out[1].words[0].end - 1.0).abs() < 1e-9,
            "{:?}",
            out[1].words
        );
        // English keeps its stream.
        assert!((out[2].words[0].end - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_reset_never_goes_back_past_the_stream_start() {
        // A Vietnamese final whose words end where the stream began: no
        // restart there (it would read the same audio forever).
        let (out, opened) = run(vec![vec![(0.5, "chào", true), (1.5, "ok", true)]], 2.0);
        let texts: Vec<&str> = out.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["chào", "ok"]);
        assert_eq!(opened, 1);
    }
}
