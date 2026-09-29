// SPDX-License-Identifier: Apache-2.0
//! `ghi diarize`: a `ghi.diarization/1` document.

use std::path::Path;

use ghi_speech::SpeakerSegment;

use crate::audio;
use crate::contract::{Diarization, ErrorDoc, Pass, Turn};
use crate::engine::EngineArgs;

pub struct Args<'a> {
    pub audio: &'a Path,
    pub pass: Pass,
    pub max_speakers: Option<u8>,
    pub engine: &'a EngineArgs,
}

pub fn run(args: &Args) -> Result<(), ErrorDoc> {
    crate::check_input_file(args.audio)?;
    let audio = audio::read_wav(args.audio)?;
    let doc = diarize(args, &audio)?;
    crate::emit(&doc)
}

#[cfg(not(feature = "nemo"))]
fn diarize(_args: &Args, _audio: &audio::Audio) -> Result<Diarization, ErrorDoc> {
    Err(crate::engine::unavailable("diarize"))
}

#[cfg(feature = "nemo")]
fn diarize(args: &Args, audio: &audio::Audio) -> Result<Diarization, ErrorDoc> {
    use crate::contract::{DIARIZATION, Perf};
    use crate::engine;

    let started = std::time::Instant::now();
    let (diar, engine_info) = engine::load_diar(args.engine, args.pass)?;
    if let Some(max) = args.max_speakers
        && u32::from(max) > diar.num_speakers()
    {
        crate::warn(&format!(
            "ghi: --max-speakers {max} exceeds the model's {} speakers",
            diar.num_speakers()
        ));
    }
    let mut stream = diar.stream().map_err(engine::speech_error)?;
    let segments = crate::cmd::feed_diar(&mut stream, audio).map_err(engine::speech_error)?;
    let wall_s = started.elapsed().as_secs_f64();
    let duration_s = audio.duration_s();
    Ok(Diarization {
        schema: DIARIZATION.to_owned(),
        audio: audio::display_name(args.audio),
        duration_s,
        pass: args.pass,
        engine: engine_info,
        turns: turns(&segments),
        perf: Perf {
            wall_s,
            rtf: (duration_s > 0.0).then(|| wall_s / duration_s),
            peak_rss_mb: engine::peak_rss_mb(),
        },
    })
}

/// Engine segments (1-based speakers) to contract turns labelled `S1`, `S2`, ...
pub fn turns(segments: &[SpeakerSegment]) -> Vec<Turn> {
    segments
        .iter()
        .filter(|s| s.end > s.start)
        .map(|s| Turn {
            start: s.start,
            end: s.end,
            speaker: format!("S{}", s.speaker),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_become_labelled_turns() {
        let t = turns(&[
            SpeakerSegment {
                start: 0.5,
                end: 2.0,
                speaker: 1,
            },
            SpeakerSegment {
                start: 2.0,
                end: 2.0,
                speaker: 2,
            },
            SpeakerSegment {
                start: 2.1,
                end: 3.0,
                speaker: 2,
            },
        ]);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].speaker.as_str(), t[1].speaker.as_str()), ("S1", "S2"));
    }
}
