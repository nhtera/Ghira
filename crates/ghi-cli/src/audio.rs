// SPDX-License-Identifier: Apache-2.0
//! WAV input: any PCM or float WAV, mixed down to mono `f32`. The engines
//! resample 8–96 kHz themselves.

use std::path::Path;

use crate::contract::{ErrorCode, ErrorDoc};

pub struct Audio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl Audio {
    pub fn duration_s(&self) -> f64 {
        self.samples.len() as f64 / f64::from(self.sample_rate)
    }
}

/// Sample rates the engines accept (they resample to the model rate).
pub const RATES: std::ops::RangeInclusive<u32> = 8_000..=96_000;

pub fn read_wav(path: &Path) -> Result<Audio, ErrorDoc> {
    let bad = |msg: String| {
        ErrorDoc::new(
            ErrorCode::BadInput,
            format!("cannot read WAV {}: {msg}", path.display()),
        )
    };
    let mut reader = hound::WavReader::open(path).map_err(|e| bad(e.to_string()))?;
    let spec = reader.spec();
    if !RATES.contains(&spec.sample_rate) {
        return Err(bad(format!(
            "sample rate {} Hz is outside 8-96 kHz",
            spec.sample_rate
        )));
    }
    let channels = usize::from(spec.channels.max(1));
    // Mix down while reading, so no interleaved copy of a long file is held
    // (it would inflate the peak RSS we report).
    let mut samples = Vec::with_capacity(reader.duration() as usize);
    let mut frame_sum = 0.0f32;
    let mut in_frame = 0usize;
    let mut push = |v: f32| {
        frame_sum += v;
        in_frame += 1;
        if in_frame == channels {
            samples.push(frame_sum / channels as f32);
            frame_sum = 0.0;
            in_frame = 0;
        }
    };
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for s in reader.samples::<f32>() {
                push(s.map_err(|e| bad(e.to_string()))?);
            }
        }
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            for s in reader.samples::<i32>() {
                push(s.map_err(|e| bad(e.to_string()))? as f32 * scale);
            }
        }
    }
    if samples.is_empty() {
        return Err(bad("no audio samples".into()));
    }
    Ok(Audio {
        samples,
        sample_rate: spec.sample_rate,
    })
}

/// File name for the `audio` field of results (never a full path).
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_wav(path: &Path, channels: u16, frames: &[[i16; 2]]) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for f in frames {
            for sample in &f[..usize::from(channels)] {
                w.write_sample(*sample).unwrap();
            }
        }
        w.finalize().unwrap();
    }

    #[test]
    fn reads_and_mixes_down_stereo() {
        let path = std::env::temp_dir().join(format!("ghi-audio-{}.wav", std::process::id()));
        write_wav(&path, 2, &[[16384, 0], [-16384, -16384]]);
        let audio = read_wav(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(audio.sample_rate, 16_000);
        assert_eq!(audio.samples, vec![0.25, -0.5]);
    }

    #[test]
    fn rejects_bad_rates_and_empty_files() {
        let dir = std::env::temp_dir();
        for (name, rate, frames) in [
            ("rate192k", 192_000u32, 4usize),
            ("rate4k", 4_000, 4),
            ("empty", 16_000, 0),
        ] {
            let path = dir.join(format!("ghi-audio-{}-{name}.wav", std::process::id()));
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut w = hound::WavWriter::create(&path, spec).unwrap();
            for _ in 0..frames {
                w.write_sample(0i16).unwrap();
            }
            w.finalize().unwrap();
            let err = read_wav(&path).err();
            let _ = std::fs::remove_file(&path);
            assert_eq!(err.map(|e| e.code), Some(ErrorCode::BadInput), "{name}");
        }
    }

    #[test]
    fn rejects_non_wav() {
        let err = read_wav(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/Cargo.toml"
        )))
        .err()
        .unwrap();
        assert_eq!(err.code, ErrorCode::BadInput);
    }
}
