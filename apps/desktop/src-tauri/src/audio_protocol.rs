// SPDX-License-Identifier: Apache-2.0
//! `ghi-audio://` — short audio spans for the webview (a speaker's 3 s sample
//! in "Name your speakers"), decrypted and decoded in memory, never written to
//! disk [RT-6].
//!
//! - A command issues a random 128-bit token for one span of one meeting
//!   (at most 30 s, valid 10 minutes). The URL carries only the token: nothing
//!   from it ever becomes a path or a gid. Unknown or expired → 404.
//! - Only the Ogg pages covering the span are read and decoded
//!   (`decode_ogg_opus_span`), served as 16 kHz mono WAV with Range support
//!   (WebKit probes `bytes=0-1` before playing) and `no-store`.
//! - A discard or delete revokes the meeting's tokens.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ghi_core::pages::page_end_granule;
use ghi_store::store::{Store, TrackKind};
use rand_core::{OsRng, RngCore};
use tauri::http::{Request, Response, StatusCode, header};

use crate::{CoreState, blocking};

const TTL: Duration = Duration::from_secs(10 * 60);
const MAX_SPAN_MS: i64 = 30_000;
const RATE: u64 = 16_000;
/// Granule units (48 kHz) per 16 kHz sample.
const GRANULE_PER_SAMPLE: u64 = 3;

#[derive(Clone)]
struct Span {
    meeting: String,
    kind: TrackKind,
    t0_ms: i64,
    t1_ms: i64,
    expires: Instant,
}

#[derive(Default)]
pub struct AudioTokens(Mutex<HashMap<String, Span>>);

impl AudioTokens {
    fn issue(&self, span: Span) -> String {
        let mut b = [0u8; 16];
        OsRng.fill_bytes(&mut b);
        let token: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        map.retain(|_, s| s.expires > now);
        map.insert(token.clone(), span);
        token
    }

    fn get(&self, token: &str) -> Option<Span> {
        let map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        map.get(token)
            .filter(|s| s.expires > Instant::now())
            .cloned()
    }

    /// After a discard or delete: the meeting's spans may be gone.
    pub fn revoke_meeting(&self, meeting: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, s| s.meeting != meeting);
    }
}

/// A token for `ghi-audio://localhost/<token>` (build the URL with
/// `convertFileSrc(token, "ghi-audio")`). `track`: 0 mic, 1 system; default
/// is the diarized track (system in a call, else the mic).
#[tauri::command]
#[specta::specta]
pub async fn issue_audio_sample(
    core: CoreState<'_>,
    tokens: tauri::State<'_, Arc<AudioTokens>>,
    meeting: String,
    t0_ms: f64,
    t1_ms: f64,
    track: Option<u32>,
) -> Result<String, String> {
    let tokens = tokens.inner().clone();
    blocking(&core, move |c| {
        let store = c.store()?;
        let m = store.get_meeting(&meeting).map_err(|e| e.to_string())?;
        let kinds: Vec<TrackKind> = store
            .tracks(&meeting)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        let want = match track {
            Some(0) => TrackKind::Mic,
            Some(_) => TrackKind::System,
            None if m.mode == "call" && kinds.contains(&TrackKind::System) => TrackKind::System,
            None => TrackKind::Mic,
        };
        // Imported files have one track of their own kind.
        let kind = if kinds.contains(&want) {
            want
        } else {
            *kinds.first().ok_or("this meeting has no audio")?
        };
        // Within the recording; NaN or huge numbers can't overflow below.
        let dur = if m.duration_ms > 0 {
            m.duration_ms
        } else {
            i64::MAX / 4
        };
        let ms = |v: f64| {
            if v.is_finite() {
                (v.max(0.0) as i64).min(dur)
            } else {
                0
            }
        };
        let t0 = ms(t0_ms);
        let t1 = ms(t1_ms).clamp(t0, t0.saturating_add(MAX_SPAN_MS));
        if t1 <= t0 {
            return Err("empty span".into());
        }
        Ok(tokens.issue(Span {
            meeting,
            kind,
            t0_ms: t0,
            t1_ms: t1,
            expires: Instant::now() + TTL,
        }))
    })
    .await
}

/// 16 kHz mono samples of the span (decrypting and decoding only its pages).
fn decode_span(store: &Store, s: &Span) -> Result<Vec<f32>, String> {
    let err = |e: ghi_store::StoreError| e.to_string();
    let reader = store.open_bundle(&s.meeting, s.kind).map_err(err)?;
    let n = reader.page_count();
    if n < 2 {
        return Err("no audio".into());
    }
    // Record 0 holds the header pages; audio records follow in time order.
    let header = reader.page(0).map_err(err)?;
    // A page without a granule (-1: a packet continues) ends where the page
    // before it did; look back a few records to keep the search ordered.
    let end_of = |i: u32| -> Result<u64, String> {
        for j in (1..=i).rev().take(8) {
            if let Some(g) = page_end_granule(&reader.page(j).map_err(err)?) {
                return Ok(g);
            }
        }
        Ok(0)
    };
    let to_granule = |ms: i64| (ms.max(0) as u64).saturating_mul(RATE) / 1000 * GRANULE_PER_SAMPLE;
    let (want0, want1) = (to_granule(s.t0_ms), to_granule(s.t1_ms));
    // First record whose audio ends after the span start (binary search).
    let (mut lo, mut hi) = (1u32, n);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if end_of(mid)? <= want0 {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    // One record early as decoder warm-up.
    let first = lo.saturating_sub(1).max(1);
    let origin = if first > 1 { end_of(first - 1)? } else { 0 };
    let mut bytes = header;
    let mut i = first;
    while i < n {
        let p = reader.page(i).map_err(err)?;
        let end = page_end_granule(&p).unwrap_or(0);
        bytes.extend_from_slice(&p);
        i += 1;
        if end >= want1 {
            break;
        }
    }
    let d = ghi_audio::encoder::decode_ogg_opus_span(&bytes, origin).map_err(|e| e.to_string())?;
    let at = |ms: i64| {
        ((ms.max(0) as u64).saturating_mul(RATE) / 1000)
            .saturating_sub(d.start)
            .min(d.samples.len() as u64) as usize
    };
    let (a, b) = (at(s.t0_ms), at(s.t1_ms));
    if a >= b {
        return Err("span beyond the audio".into());
    }
    Ok(d.samples[a..b].to_vec())
}

fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&(RATE as u32).to_le_bytes());
    out.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &x in samples {
        out.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// `bytes=a-b` / `bytes=a-` → the inclusive range within `len`.
fn range(header: Option<&str>, len: usize) -> Option<(usize, usize)> {
    let spec = header?.strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    if a.trim().is_empty() {
        // Suffix: the last n bytes.
        let n: usize = b.trim().parse().ok()?;
        return (n > 0 && len > 0).then(|| (len.saturating_sub(n), len - 1));
    }
    let start: usize = a.trim().parse().ok()?;
    let end = if b.trim().is_empty() {
        len.checked_sub(1)?
    } else {
        b.trim().parse::<usize>().ok()?.min(len.checked_sub(1)?)
    };
    (start <= end).then_some((start, end))
}

pub fn server_error() -> Response<Vec<u8>> {
    plain(StatusCode::INTERNAL_SERVER_ERROR)
}

fn plain(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .body(Vec::new())
        .expect("static response")
}

/// The protocol handler: token → WAV bytes (with Range).
pub fn respond(
    store: Option<Arc<Store>>,
    tokens: &AudioTokens,
    req: &Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    if req.method() != tauri::http::Method::GET {
        return plain(StatusCode::METHOD_NOT_ALLOWED);
    }
    let token = req.uri().path().trim_start_matches('/');
    if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return plain(StatusCode::NOT_FOUND);
    }
    let (Some(span), Some(store)) = (tokens.get(token), store) else {
        return plain(StatusCode::NOT_FOUND);
    };
    let Ok(samples) = decode_span(&store, &span) else {
        return plain(StatusCode::NOT_FOUND);
    };
    let body = wav(&samples);
    let base = |status| {
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "audio/wav")
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CACHE_CONTROL, "no-store")
    };
    let requested = req
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok());
    match requested.map(|r| range(Some(r), body.len())) {
        Some(Some((a, b))) => base(StatusCode::PARTIAL_CONTENT)
            .header(
                header::CONTENT_RANGE,
                format!("bytes {a}-{b}/{}", body.len()),
            )
            .body(body[a..=b].to_vec()),
        Some(None) => Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{}", body.len()))
            .body(Vec::new()),
        None => base(StatusCode::OK).body(body),
    }
    .unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(range(Some("bytes=0-1"), 100), Some((0, 1)));
        assert_eq!(range(Some("bytes=10-"), 100), Some((10, 99)));
        assert_eq!(range(Some("bytes=90-500"), 100), Some((90, 99)));
        assert_eq!(range(Some("bytes=200-"), 100), None);
        assert_eq!(range(Some("items=0-1"), 100), None);
        assert_eq!(range(Some("bytes=-10"), 100), Some((90, 99)));
        assert_eq!(range(Some("bytes=-500"), 100), Some((0, 99)));
    }

    #[test]
    fn wav_header_is_16k_mono_pcm() {
        let w = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 6);
        assert_eq!(w.len(), 44 + 6);
    }

    #[test]
    fn tokens_expire_and_are_revoked_per_meeting() {
        let t = AudioTokens::default();
        let span = |m: &str, expires| Span {
            meeting: m.into(),
            kind: TrackKind::Mic,
            t0_ms: 0,
            t1_ms: 1000,
            expires,
        };
        let a = t.issue(span("m1", Instant::now() + TTL));
        let b = t.issue(span("m2", Instant::now() + TTL));
        let old = t.issue(span("m2", Instant::now() - Duration::from_secs(1)));
        assert_eq!(a.len(), 32);
        assert!(t.get(&a).is_some() && t.get(&b).is_some());
        assert!(t.get(&old).is_none(), "expired");
        t.revoke_meeting("m1");
        assert!(t.get(&a).is_none() && t.get(&b).is_some());
    }

    #[test]
    fn only_tokens_reach_the_store() {
        let t = AudioTokens::default();
        let req = |uri: &str, method: &str| {
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Vec::new())
                .unwrap()
        };
        // No store needed to refuse these.
        assert_eq!(
            respond(
                None,
                &t,
                &req("ghi-audio://localhost/../../etc/passwd", "GET")
            )
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            respond(
                None,
                &t,
                &req(
                    "ghi-audio://localhost/0123456789abcdef0123456789abcdef",
                    "GET"
                )
            )
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            respond(None, &t, &req("ghi-audio://localhost/x", "POST")).status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
}
