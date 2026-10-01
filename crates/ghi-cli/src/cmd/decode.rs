// SPDX-License-Identifier: Apache-2.0
//! `ghi decode`: probe or decode an audio (or video) file with the import
//! decoders (phase 8). Dev and test tooling for `ghi_audio::decode`.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::Instant;

use ghi_audio::SAMPLE_RATE;
use ghi_audio::decode::{DecodeError, Decoder, Info};
use serde_json::{Value, json};

use crate::contract::{ErrorCode, ErrorDoc};

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    pub audio: PathBuf,
    /// Print the file's properties without decoding it.
    #[arg(long, conflicts_with_all = ["wav", "channel"])]
    pub probe: bool,
    /// Write the audio as 16 kHz 16-bit WAV (mono: the chosen channel or the mix).
    #[arg(long, value_name = "OUT.WAV")]
    pub wav: Option<PathBuf>,
    /// Source channel to keep, from 0 (default: the average of all channels).
    #[arg(long, value_name = "N")]
    pub channel: Option<usize>,
}

fn decode_error(e: DecodeError) -> ErrorDoc {
    let code = match e {
        DecodeError::Io(_) => ErrorCode::Internal,
        _ => ErrorCode::BadInput,
    };
    ErrorDoc::new(code, e.to_string())
}

fn info_json(i: &Info) -> Value {
    json!({
        "channels": i.channels,
        "sample_rate": i.sample_rate,
        "duration_s": i.duration_ms.map(|ms| ms as f64 / 1000.0),
        "codec": i.codec,
        "container": i.container,
        "backend": i.backend.as_str(),
    })
}

/// Probe-only, or a full decode that optionally writes a WAV. Either way prints
/// one `ghi.decode/1` document; a decode adds `output` (frames, duration) and
/// `perf` (`speed_x`: seconds of audio decoded per second of wall time).
pub fn run(args: &Args) -> Result<(), ErrorDoc> {
    let file = args.audio.display().to_string();
    let mut dec = Decoder::open(&args.audio).map_err(decode_error)?;
    let info = dec.info().clone();
    if args.probe {
        return crate::emit(
            &json!({"schema": "ghi.decode/1", "file": file, "info": info_json(&info)}),
        );
    }
    if let Some(c) = args.channel.filter(|&c| c >= info.channels as usize) {
        return Err(ErrorDoc::new(
            ErrorCode::BadInput,
            format!(
                "--channel {c}: the file has {} channel(s) (0-based)",
                info.channels
            ),
        ));
    }

    let mut writer = match &args.wav {
        Some(path) => {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let out = File::create(path).map_err(|e| {
                ErrorDoc::new(
                    ErrorCode::BadInput,
                    format!("--wav {}: {e}", path.display()),
                )
            })?;
            Some(hound::WavWriter::new(BufWriter::new(out), spec).map_err(internal)?)
        }
        None => None,
    };

    let started = Instant::now();
    let mut frames = 0u64;
    let mut peak = 0f32;
    while let Some(block) = dec.next_block().map_err(decode_error)? {
        frames += block.frames() as u64;
        for i in 0..block.frames() {
            let v = match args.channel {
                Some(c) => block.channels[c][i],
                None => {
                    block.channels.iter().map(|c| c[i]).sum::<f32>() / block.channels.len() as f32
                }
            };
            peak = peak.max(v.abs());
            if let Some(w) = writer.as_mut() {
                w.write_sample((v.clamp(-1.0, 1.0) * 32767.0).round() as i16)
                    .map_err(internal)?;
            }
        }
    }
    let wall_s = started.elapsed().as_secs_f64();
    if let Some(w) = writer {
        w.finalize().map_err(internal)?;
    }
    let audio_s = frames as f64 / SAMPLE_RATE as f64;
    crate::emit(&json!({
        "schema": "ghi.decode/1",
        "file": file,
        "info": info_json(&info),
        "output": {
            "wav": args.wav.as_ref().map(|p| p.display().to_string()),
            "channel": args.channel,
            "sample_rate": SAMPLE_RATE,
            "frames": frames,
            "duration_s": audio_s,
            "peak": peak,
        },
        "perf": {"wall_s": wall_s, "speed_x": (wall_s > 0.0).then(|| audio_s / wall_s)},
    }))
}

fn internal(e: impl std::fmt::Display) -> ErrorDoc {
    ErrorDoc::new(ErrorCode::Internal, e.to_string())
}
