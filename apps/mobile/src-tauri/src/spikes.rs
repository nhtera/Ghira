// SPDX-License-Identifier: Apache-2.0
//! Simulator spikes (phase 16-B), compiled only with the `test-hooks` feature.
//!
//! `GHI_SPIKE=audio` (via `SIMCTL_CHILD_GHI_SPIKE`): serves a generated WAV
//! over the custom scheme `ghi-audio://` with HTTP range support and injects
//! an `<audio>` element into the webview that plays, seeks and reports on
//! screen. Every request (with its `Range` header) goes to stderr.

use std::sync::Mutex;

use tauri::http::{self, Request, Response};

const RATE: u32 = 16_000;
const SECONDS: u32 = 20;

/// A 20 s, 16 kHz mono 16-bit PCM WAV (a 440 Hz tone), built once.
fn wav() -> &'static [u8] {
    static WAV: Mutex<Option<&'static [u8]>> = Mutex::new(None);
    let mut g = WAV.lock().unwrap();
    if let Some(w) = *g {
        return w;
    }
    let n = RATE * SECONDS;
    let mut v = Vec::with_capacity(44 + 2 * n as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + 2 * n).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&RATE.to_le_bytes());
    v.extend_from_slice(&(RATE * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(2 * n).to_le_bytes());
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        let s = (t * 440.0 * std::f32::consts::TAU).sin() * 0.2;
        v.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
    }
    let w: &'static [u8] = Box::leak(v.into_boxed_slice());
    *g = Some(w);
    w
}

fn parse_range(h: &str, len: usize) -> Option<(usize, usize)> {
    let spec = h.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    let start: usize = a.parse().ok()?;
    let end: usize = if b.is_empty() {
        len - 1
    } else {
        b.parse().ok()?
    };
    (start <= end && start < len).then_some((start, end.min(len - 1)))
}

pub fn audio_scheme(
    _ctx: tauri::UriSchemeContext<'_, tauri::Wry>,
    req: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let body = wav();
    let range = req
        .headers()
        .get(http::header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut res = Response::builder()
        .header(http::header::CONTENT_TYPE, "audio/wav")
        .header(http::header::ACCEPT_RANGES, "bytes");
    let out = match range.as_deref().and_then(|h| parse_range(h, body.len())) {
        Some((a, b)) => {
            res = res.status(206).header(
                http::header::CONTENT_RANGE,
                format!("bytes {a}-{b}/{}", body.len()),
            );
            body[a..=b].to_vec()
        }
        None => {
            res = res.status(200);
            body.to_vec()
        }
    };
    eprintln!(
        "ghira: spike a: {} {} range={:?} -> {} bytes",
        req.method(),
        req.uri(),
        range,
        out.len()
    );
    res.header(http::header::CONTENT_LENGTH, out.len())
        .body(out)
        .unwrap()
}

/// Injected into the page when `GHI_SPIKE=audio`: plays the tone, seeks to
/// the middle (which needs a range request) and shows the outcome on screen.
pub const AUDIO_SCRIPT: &str = r#"
window.addEventListener('DOMContentLoaded', () => {
  const box = document.createElement('pre');
  box.id = 'spike-a';
  box.style.cssText = 'position:fixed;left:0;right:0;bottom:0;z-index:99999;background:#000;color:#0f0;font:14px monospace;padding:8px;margin:0;white-space:pre-wrap';
  document.body.appendChild(box);
  const log = (m) => { box.textContent += m + '\n'; };
  const a = new Audio('ghi-audio://localhost/tone.wav');
  a.preload = 'auto';
  for (const e of ['loadedmetadata','canplay','playing','seeked','error','stalled'])
    a.addEventListener(e, () => log(e + ' t=' + a.currentTime.toFixed(2) + ' dur=' + (a.duration||0).toFixed(1) + (a.error ? ' err=' + a.error.code : '')));
  a.muted = true;
  a.play().then(() => log('play() ok'), (e) => log('play() rejected: ' + e));
  setTimeout(() => { a.currentTime = 10; }, 2500);
  setTimeout(() => log('final t=' + a.currentTime.toFixed(2) + ' paused=' + a.paused + ' ended=' + a.ended), 6000);
});
"#;
