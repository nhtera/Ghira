// SPDX-License-Identifier: Apache-2.0
//! `ghi detect`: lists the processes using audio and runs the meeting
//! auto-detect policy on them, once or every second (`--watch`).

use std::time::{Duration, Instant, SystemTime};

use ghi_audio::detect::{App, AudioProcess, DetectConfig, DetectState, Detector, Prompt};

use crate::contract::{DETECT, Detect, DetectProcess, DetectPrompt, ErrorCode, ErrorDoc};

fn processes() -> Result<Vec<AudioProcess>, ErrorDoc> {
    #[cfg(target_os = "macos")]
    {
        ghi_audio::macos::audio_processes()
            .map_err(|e| ErrorDoc::new(ErrorCode::Internal, e.to_string()))
    }
    #[cfg(not(target_os = "macos"))]
    Err(ErrorDoc::new(
        ErrorCode::CaptureUnavailable,
        "detect: macOS-only for now (Windows in phase 13)",
    ))
}

fn doc(procs: &[AudioProcess], prompt: Option<Prompt>) -> Detect {
    Detect {
        schema: DETECT.into(),
        processes: procs
            .iter()
            .map(|p| DetectProcess {
                pid: p.pid,
                bundle_id: p.bundle_id.clone(),
                input: p.input,
                output: p.output,
                app: App::from_bundle_id(&p.bundle_id).map(|a| a.key().to_owned()),
            })
            .collect(),
        prompt: prompt.map(|p| DetectPrompt {
            app: p.app.key().into(),
            title: p.title.into(),
            pids: p.pids,
        }),
    }
}

/// Without `watch_s`: one document for the current processes (a single poll,
/// so the "mic running for 2 polls" filter is off). With it: polls every
/// second for that long and prints a document (NDJSON) per prompt.
pub fn run(watch_s: Option<u64>) -> Result<(), ErrorDoc> {
    let Some(watch_s) = watch_s else {
        let procs = processes()?;
        let cfg = DetectConfig {
            stable_polls: 1,
            ..DetectConfig::default()
        };
        let prompt = Detector::new(cfg, DetectState::default()).poll(&procs, SystemTime::now());
        return crate::emit(&doc(&procs, prompt));
    };
    let mut detector = Detector::new(DetectConfig::default(), DetectState::default());
    let end = Instant::now() + Duration::from_secs(watch_s);
    while Instant::now() < end {
        let procs = processes()?;
        if let Some(prompt) = detector.poll(&procs, SystemTime::now()) {
            crate::emit(&doc(&procs, Some(prompt)))?;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}
