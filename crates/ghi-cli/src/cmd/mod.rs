// SPDX-License-Identifier: Apache-2.0
//! Engine commands. The feeding and result-shaping logic is generic over the
//! `ghi-speech` stream traits so it is testable without models.

pub mod bench;
pub mod detect;
pub mod diarize;
pub mod record;
pub mod transcribe;

use std::time::{Duration, Instant};

use ghi_speech::{AsrResult, AsrStream, DiarStream, SpeakerSegment};

use crate::audio::Audio;

/// Audio is pushed in 100 ms blocks, like the capture pipeline.
const BLOCK_S: f64 = 0.1;

/// Feeds `audio` into an ASR stream and hands every result to `sink` with the
/// seconds elapsed since feeding started; `sink` returns `false` to stop early
/// (e.g. stdout closed). With `realtime`, each block is pushed only once that
/// much audio could have been captured (1× speed).
pub fn feed_asr<S: AsrStream>(
    stream: &mut S,
    audio: &Audio,
    realtime: bool,
    mut sink: impl FnMut(AsrResult, f64) -> bool,
) -> ghi_speech::Result<()> {
    let start = Instant::now();
    let block = ((f64::from(audio.sample_rate) * BLOCK_S) as usize).max(1);
    for (i, chunk) in audio.samples.chunks(block).enumerate() {
        if realtime {
            let due = Duration::from_secs_f64(
                (i + 1) as f64 * block as f64 / f64::from(audio.sample_rate),
            );
            if let Some(wait) = due.checked_sub(start.elapsed()) {
                std::thread::sleep(wait);
            }
        }
        stream.push(chunk, audio.sample_rate)?;
        while let Some(r) = stream.next_result()? {
            if !sink(r, start.elapsed().as_secs_f64()) {
                return Ok(());
            }
        }
    }
    stream.finish()?;
    while let Some(r) = stream.next_result()? {
        if !sink(r, start.elapsed().as_secs_f64()) {
            return Ok(());
        }
    }
    Ok(())
}

/// Feeds all of `audio` into a diarization stream and returns its segments.
pub fn feed_diar<S: DiarStream>(
    stream: &mut S,
    audio: &Audio,
) -> ghi_speech::Result<Vec<SpeakerSegment>> {
    let block = ((f64::from(audio.sample_rate) * BLOCK_S) as usize).max(1);
    for chunk in audio.samples.chunks(block) {
        stream.push(chunk, audio.sample_rate)?;
    }
    stream.finish()?;
    stream.segments()
}

#[cfg(test)]
pub mod fake {
    //! Scripted engines for tests.
    use ghi_speech::{AsrResult, AsrStream, DiarStream, Result, SpeakerSegment, Word};

    pub fn word(text: &str, start: f64, end: f64) -> Word {
        Word {
            text: text.into(),
            start,
            end,
            confidence: 1.0,
            speaker: None,
        }
    }

    pub fn result(is_final: bool, lang: &str, words: Vec<Word>) -> AsrResult {
        AsrResult {
            is_final,
            text: words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            audio_processed: words.last().map_or(0.0, |w| w.end),
            words,
            languages: vec![lang.into()],
        }
    }

    /// Returns one scripted result after each push, then the rest after finish.
    #[derive(Default)]
    pub struct FakeAsr {
        pub on_push: Vec<AsrResult>,
        pub on_finish: Vec<AsrResult>,
        pub pushed: usize,
        pub ready: Vec<AsrResult>,
    }

    impl AsrStream for FakeAsr {
        fn push(&mut self, pcm: &[f32], _rate: u32) -> Result<()> {
            self.pushed += pcm.len();
            if !self.on_push.is_empty() {
                self.ready.push(self.on_push.remove(0));
            }
            Ok(())
        }
        fn finish(&mut self) -> Result<()> {
            self.ready.append(&mut self.on_finish);
            Ok(())
        }
        fn next_result(&mut self) -> Result<Option<AsrResult>> {
            Ok((!self.ready.is_empty()).then(|| self.ready.remove(0)))
        }
    }

    #[derive(Default)]
    pub struct FakeDiar {
        pub pushed: usize,
        pub finished: bool,
        pub segments: Vec<SpeakerSegment>,
    }

    impl DiarStream for FakeDiar {
        fn push(&mut self, pcm: &[f32], _rate: u32) -> Result<()> {
            self.pushed += pcm.len();
            Ok(())
        }
        fn finish(&mut self) -> Result<()> {
            self.finished = true;
            Ok(())
        }
        fn segments(&self) -> Result<Vec<SpeakerSegment>> {
            assert!(self.finished, "segments read before finish");
            Ok(self.segments.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    fn audio(seconds: f64) -> Audio {
        Audio {
            samples: vec![0.0; (16_000.0 * seconds) as usize],
            sample_rate: 16_000,
        }
    }

    #[test]
    fn feeds_everything_and_drains_after_finish() {
        let mut asr = FakeAsr {
            on_push: vec![result(false, "vi-VN", vec![word("xin", 0.0, 0.2)])],
            on_finish: vec![result(
                true,
                "vi-VN",
                vec![word("xin", 0.0, 0.2), word("chào", 0.3, 0.5)],
            )],
            ..Default::default()
        };
        let mut got = Vec::new();
        feed_asr(&mut asr, &audio(0.35), false, |r, _| {
            got.push(r.is_final);
            true
        })
        .unwrap();
        assert_eq!(asr.pushed, 5600);
        assert_eq!(got, vec![false, true]);
    }

    #[test]
    fn realtime_takes_at_least_the_audio_duration() {
        let mut asr = FakeAsr::default();
        let t = Instant::now();
        feed_asr(&mut asr, &audio(0.3), true, |_, _| true).unwrap();
        assert!(t.elapsed().as_secs_f64() >= 0.29);
    }

    #[test]
    fn sink_can_stop_feeding() {
        let mut asr = FakeAsr {
            on_push: vec![result(false, "vi-VN", vec![word("a", 0.0, 0.1)]); 5],
            ..Default::default()
        };
        let mut seen = 0;
        feed_asr(&mut asr, &audio(0.5), false, |_, _| {
            seen += 1;
            false
        })
        .unwrap();
        assert_eq!((seen, asr.pushed), (1, 1600));
    }

    #[test]
    fn diar_finishes_before_reading_segments() {
        let mut diar = FakeDiar::default();
        feed_diar(&mut diar, &audio(0.25)).unwrap();
        assert_eq!(diar.pushed, 4000);
    }
}
