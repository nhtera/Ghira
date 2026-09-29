// SPDX-License-Identifier: Apache-2.0
//! `ghi bench`: runs the speech engines on one file and reports stage timings
//! (`ghi.bench/1`). `--topology call` runs the call topology of RT-11 in one
//! process: ASR and diarization on both the mic and the system track, all
//! concurrently (the same file stands in for both tracks). One loaded model
//! of each kind serves both tracks. AEC joins in phase 4.

use std::path::Path;

use crate::audio;
use crate::contract::{ErrorDoc, Pass};
use crate::engine::EngineArgs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Topology {
    /// One ASR stream, then diarization.
    Single,
    /// Two ASR streams and diarization at the same time.
    Call,
}

pub struct Args<'a> {
    pub audio: &'a Path,
    pub pass: Pass,
    pub topology: Topology,
    pub engine: &'a EngineArgs,
}

pub fn run(args: &Args) -> Result<(), ErrorDoc> {
    crate::check_input_file(args.audio)?;
    let audio = audio::read_wav(args.audio)?;
    let doc = bench(args, &audio)?;
    crate::emit(&doc)
}

#[cfg(not(feature = "nemo"))]
fn bench(_args: &Args, _audio: &audio::Audio) -> Result<crate::contract::Bench, ErrorDoc> {
    Err(crate::engine::unavailable("bench"))
}

#[cfg(feature = "nemo")]
fn bench(args: &Args, audio: &audio::Audio) -> Result<crate::contract::Bench, ErrorDoc> {
    use std::time::Instant;

    use ghi_speech::nemo::{Asr, AsrOptions, Diarizer};

    use crate::contract::{BENCH, Bench, Perf, Stage};
    use crate::engine;

    fn asr_stage(asr: &Asr, audio: &audio::Audio, name: &str) -> ghi_speech::Result<Stage> {
        let t = Instant::now();
        let mut stream = asr.stream(&AsrOptions::default())?;
        crate::cmd::feed_asr(&mut stream, audio, false, |_, _| true)?;
        Ok(Stage {
            name: name.into(),
            wall_s: t.elapsed().as_secs_f64(),
        })
    }
    fn diar_stage(diar: &Diarizer, audio: &audio::Audio, name: &str) -> ghi_speech::Result<Stage> {
        let t = Instant::now();
        let mut stream = diar.stream()?;
        crate::cmd::feed_diar(&mut stream, audio)?;
        Ok(Stage {
            name: name.into(),
            wall_s: t.elapsed().as_secs_f64(),
        })
    }

    let t_load = Instant::now();
    let (asr, _) = engine::load_asr(args.engine, args.pass)?;
    let (diar, _) = engine::load_diar(args.engine, args.pass)?;
    let mut stages = vec![Stage {
        name: "load".into(),
        wall_s: t_load.elapsed().as_secs_f64(),
    }];

    let t_run = Instant::now();
    let run: ghi_speech::Result<Vec<Stage>> = match args.topology {
        Topology::Single => (|| {
            Ok(vec![
                asr_stage(&asr, audio, "asr")?,
                diar_stage(&diar, audio, "diarization")?,
            ])
        })(),
        Topology::Call => std::thread::scope(|s| {
            let handles = [
                s.spawn(|| asr_stage(&asr, audio, "asr-mic")),
                s.spawn(|| asr_stage(&asr, audio, "asr-system")),
                s.spawn(|| diar_stage(&diar, audio, "diarization-mic")),
                s.spawn(|| diar_stage(&diar, audio, "diarization-system")),
            ];
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        Err(ghi_speech::SpeechError {
                            op: "bench",
                            message: "a worker thread panicked".into(),
                        })
                    })
                })
                .collect()
        }),
    };
    stages.extend(run.map_err(engine::speech_error)?);
    // RTF excludes model load: it is the steady-state processing cost.
    let wall_s = t_run.elapsed().as_secs_f64();
    let duration_s = audio.duration_s();
    Ok(Bench {
        schema: BENCH.to_owned(),
        audio: audio::display_name(args.audio),
        duration_s,
        pass: args.pass,
        stages,
        perf: Perf {
            wall_s,
            rtf: (duration_s > 0.0).then(|| wall_s / duration_s),
            peak_rss_mb: engine::peak_rss_mb(),
        },
    })
}
