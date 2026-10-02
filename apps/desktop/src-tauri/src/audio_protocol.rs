// SPDX-License-Identifier: Apache-2.0
//! `ghi-audio://` — meeting audio for the webview, decrypted and decoded in
//! memory, never written to disk [RT-6].
//!
//! - A command issues a random 128-bit token; the URL carries only the token:
//!   nothing from it ever becomes a path or a gid. Unknown or expired → 404.
//! - A *sample* token is one span of at most 30 s ("Name your speakers"),
//!   valid 10 minutes.
//! - A *play* token is the whole meeting for the audio bar (D6): a virtual
//!   16 kHz mono PCM WAV whose length is fixed when issued, so seeking is a
//!   byte range. Each response decodes only the Ogg pages it covers (at most
//!   1 MiB, ~32 s; media elements keep asking for the next range), with both
//!   tracks of a call mixed. Gaps (discarded spans) play as silence, so
//!   bytes map to meeting time exactly. It stays valid while in use, and only
//!   the latest play token works (one meeting is open at a time).
//! - Responses support Range (WebKit probes `bytes=0-1` first) and are
//!   `no-store`. A discard or delete revokes the meeting's tokens.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ghi_core::pages::page_end_granule;
use ghi_store::store::{Store, TrackKind};
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use specta::Type;
use tauri::http::{Request, Response, StatusCode, header};

use crate::{CoreState, blocking};

const TTL: Duration = Duration::from_secs(10 * 60);
const MAX_SPAN_MS: i64 = 30_000;
const RATE: u64 = 16_000;
/// Granule units (48 kHz) per 16 kHz sample.
const GRANULE_PER_SAMPLE: u64 = 3;
const WAV_HEADER: u64 = 44;
/// Gain on a mix of two tracks.
const HEADROOM: f32 = 0.7;
/// Largest body of one play response.
const MAX_CHUNK: u64 = 1024 * 1024;
/// Waveform resolution: one loudness value per 100 ms.
const PEAKS_PER_SECOND: u64 = 10;

#[derive(Clone, Debug, PartialEq)]
enum Grant {
    /// A short span of one track.
    Sample {
        kind: TrackKind,
        t0_ms: i64,
        t1_ms: i64,
    },
    /// The whole meeting, its tracks mixed; `samples` long.
    Play { kinds: Vec<TrackKind>, samples: u64 },
}

#[derive(Clone)]
struct Span {
    meeting: String,
    grant: Grant,
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
        if matches!(span.grant, Grant::Play { .. }) {
            map.retain(|_, s| !matches!(s.grant, Grant::Play { .. }));
        }
        map.insert(token.clone(), span);
        token
    }

    /// A valid token's span; a play token's life is extended by each use.
    fn get(&self, token: &str) -> Option<Span> {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let s = map.get_mut(token).filter(|s| s.expires > now)?;
        if matches!(s.grant, Grant::Play { .. }) {
            s.expires = now + TTL;
        }
        Some(s.clone())
    }

    /// After a discard or delete: the meeting's spans may be gone.
    pub fn revoke_meeting(&self, meeting: &str) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, s| s.meeting != meeting);
    }
}

/// The meeting's track kinds, in bundle order.
fn kinds_of(store: &Store, meeting: &str) -> Result<Vec<TrackKind>, String> {
    Ok(store
        .tracks(meeting)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(k, _)| k)
        .collect())
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
        let kinds = kinds_of(&store, &meeting)?;
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
            grant: Grant::Sample {
                kind,
                t0_ms: t0,
                t1_ms: t1,
            },
            expires: Instant::now() + TTL,
        }))
    })
    .await
}

/// The audio bar's source: a play token and how long the audio is.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AudioPlay {
    pub token: String,
    pub duration_ms: f64,
}

/// Length of a track in 16 kHz samples (its last page's end granule).
fn track_samples(store: &Store, meeting: &str, kind: TrackKind) -> Result<u64, String> {
    let err = |e: ghi_store::StoreError| e.to_string();
    let reader = store.open_bundle(meeting, kind).map_err(err)?;
    let n = reader.page_count();
    for i in (1..n).rev().take(8) {
        if let Some(g) = page_end_granule(&reader.page(i).map_err(err)?) {
            return Ok(g / GRANULE_PER_SAMPLE);
        }
    }
    Ok(0)
}

/// Plays the whole meeting (the open one; any earlier play token stops working).
#[tauri::command]
#[specta::specta]
pub async fn issue_audio_play(
    core: CoreState<'_>,
    tokens: tauri::State<'_, Arc<AudioTokens>>,
    meeting: String,
) -> Result<AudioPlay, String> {
    let tokens = tokens.inner().clone();
    blocking(&core, move |c| {
        let store = c.store()?;
        let m = store.get_meeting(&meeting).map_err(|e| e.to_string())?;
        if m.status == "recording" || m.status == ghi_core::import::IMPORTING {
            return Err("the audio is still being written".into());
        }
        let kinds = kinds_of(&store, &meeting)?;
        if kinds.is_empty() {
            return Err("this meeting has no audio".into());
        }
        let mut samples = (m.duration_ms.max(0) as u64).saturating_mul(RATE) / 1000;
        for &k in &kinds {
            samples = samples.max(track_samples(&store, &meeting, k)?);
        }
        if samples == 0 {
            return Err("this meeting has no audio".into());
        }
        let token = tokens.issue(Span {
            meeting,
            grant: Grant::Play { kinds, samples },
            expires: Instant::now() + TTL,
        });
        Ok(AudioPlay {
            token,
            duration_ms: (samples * 1000 / RATE) as f64,
        })
    })
    .await
}

/// Samples `s0..s1` (16 kHz) of one track, decrypting and decoding only the
/// pages that cover them; silence where there is no audio. Exactly
/// `s1 - s0` samples.
fn decode_track(
    store: &Store,
    meeting: &str,
    kind: TrackKind,
    s0: u64,
    s1: u64,
) -> Result<Vec<f32>, String> {
    let len = s1.saturating_sub(s0) as usize;
    let mut out = vec![0f32; len];
    if len == 0 {
        return Ok(out);
    }
    let err = |e: ghi_store::StoreError| e.to_string();
    let reader = store.open_bundle(meeting, kind).map_err(err)?;
    let n = reader.page_count();
    if n < 2 {
        return Ok(out);
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
    let (want0, want1) = (s0 * GRANULE_PER_SAMPLE, s1 * GRANULE_PER_SAMPLE);
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
    if lo >= n {
        return Ok(out); // past the end of this track
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
    // Copy the overlap of the decoded audio with s0..s1.
    let (d0, d1) = (d.start, d.start + d.samples.len() as u64);
    let (a, b) = (s0.max(d0), s1.min(d1));
    if a < b {
        out[(a - s0) as usize..(b - s0) as usize]
            .copy_from_slice(&d.samples[(a - d0) as usize..(b - d0) as usize]);
    }
    Ok(out)
}

/// Samples `s0..s1` of the meeting with its tracks mixed.
fn decode_mix(
    store: &Store,
    meeting: &str,
    kinds: &[TrackKind],
    s0: u64,
    s1: u64,
) -> Result<Vec<f32>, String> {
    let mut mix: Option<Vec<f32>> = None;
    for &k in kinds {
        let t = decode_track(store, meeting, k, s0, s1)?;
        match &mut mix {
            None => mix = Some(t),
            Some(m) => m.iter_mut().zip(t).for_each(|(a, b)| *a += b),
        }
    }
    let mut mix = mix.unwrap_or_default();
    // Two loud sides at once would clip: leave headroom.
    if kinds.len() > 1 {
        mix.iter_mut().for_each(|x| *x *= HEADROOM);
    }
    Ok(mix)
}

/// 16 kHz mono samples of a sample span.
fn decode_span(
    store: &Store,
    meeting: &str,
    kind: TrackKind,
    t0_ms: i64,
    t1_ms: i64,
) -> Result<Vec<f32>, String> {
    if store
        .open_bundle(meeting, kind)
        .map_err(|e| e.to_string())?
        .page_count()
        < 2
    {
        return Err("no audio".into());
    }
    let at = |ms: i64| (ms.max(0) as u64).saturating_mul(RATE) / 1000;
    decode_track(store, meeting, kind, at(t0_ms), at(t1_ms))
}

fn wav_header(samples: u64) -> [u8; WAV_HEADER as usize] {
    // RIFF sizes are 32-bit: 4 GiB of 16 kHz PCM is ~37 hours.
    let data_len = u32::try_from(samples * 2).unwrap_or(u32::MAX - 36);
    let mut h = [0u8; WAV_HEADER as usize];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data_len).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    h[24..28].copy_from_slice(&(RATE as u32).to_le_bytes());
    h[28..32].copy_from_slice(&(RATE as u32 * 2).to_le_bytes());
    h[32..34].copy_from_slice(&2u16.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_len.to_le_bytes());
    h
}

fn pcm16(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &x in samples {
        out.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

fn wav(samples: &[f32]) -> Vec<u8> {
    let mut out = wav_header(samples.len() as u64).to_vec();
    out.extend(pcm16(samples));
    out
}

/// Bytes `a..=b` of the meeting's virtual WAV (`samples` long).
fn play_bytes(
    store: &Store,
    meeting: &str,
    kinds: &[TrackKind],
    samples: u64,
    a: u64,
    b: u64,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity((b - a + 1) as usize);
    if a < WAV_HEADER {
        let h = wav_header(samples);
        out.extend_from_slice(&h[a as usize..=(b.min(WAV_HEADER - 1)) as usize]);
    }
    if b >= WAV_HEADER {
        let (d0, d1) = (a.max(WAV_HEADER) - WAV_HEADER, b - WAV_HEADER);
        let (s0, s1) = (d0 / 2, (d1 / 2 + 1).min(samples));
        let pcm = pcm16(&decode_mix(store, meeting, kinds, s0, s1)?);
        let skip = (d0 - s0 * 2) as usize;
        let take = (d1 - d0 + 1) as usize;
        out.extend_from_slice(&pcm[skip..(skip + take).min(pcm.len())]);
    }
    Ok(out)
}

/// Loudness per 100 ms of the meeting's mixed tracks (0 silent .. 255 full
/// scale, on a 60 dB log scale), decoded a minute at a time.
fn compute_waveform(
    store: &Store,
    meeting: &str,
    kinds: &[TrackKind],
    samples: u64,
) -> Result<Vec<u8>, String> {
    let bucket = RATE / PEAKS_PER_SECOND;
    let window = bucket * 600;
    let mut out = Vec::with_capacity((samples / bucket + 1) as usize);
    let mut s0 = 0;
    while s0 < samples {
        let s1 = (s0 + window).min(samples);
        let pcm = decode_mix(store, meeting, kinds, s0, s1)?;
        for chunk in pcm.chunks(bucket as usize) {
            let peak = chunk.iter().fold(0f32, |m, x| m.max(x.abs())).min(1.0);
            let db = 20.0 * peak.max(1e-6).log10();
            out.push((((db + 60.0) / 60.0).clamp(0.0, 1.0) * 255.0).round() as u8);
        }
        s0 = s1;
    }
    Ok(out)
}

/// The audio bar's waveform.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Waveform {
    pub per_second: u32,
    /// Loudness 0..255 per bucket, from the start of the meeting.
    pub peaks: Vec<u8>,
}

/// The waveform of a finished meeting (computed once, then kept sealed in
/// the store until the audio goes).
#[tauri::command]
#[specta::specta]
pub async fn waveform_peaks(core: CoreState<'_>, meeting: String) -> Result<Waveform, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let wave = |peaks| Waveform {
            per_second: PEAKS_PER_SECOND as u32,
            peaks,
        };
        let m = store.get_meeting(&meeting).map_err(|e| e.to_string())?;
        // Still being written: no waveform yet.
        if m.status == "recording" || m.status == ghi_core::import::IMPORTING {
            return Err("the audio is still being written".into());
        }
        // One at a time: a second request for the same meeting waits and
        // then finds it cached.
        static COMPUTING: Mutex<()> = Mutex::new(());
        let _one = COMPUTING.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = store.waveform(&meeting).map_err(|e| e.to_string())? {
            return Ok(wave(p));
        }
        let kinds = kinds_of(&store, &meeting)?;
        if kinds.is_empty() {
            return Ok(wave(Vec::new()));
        }
        let mut samples = (m.duration_ms.max(0) as u64).saturating_mul(RATE) / 1000;
        for &k in &kinds {
            samples = samples.max(track_samples(&store, &meeting, k)?);
        }
        let peaks = compute_waveform(&store, &meeting, &kinds, samples)?;
        store
            .set_waveform(&meeting, &peaks)
            .map_err(|e| e.to_string())?;
        Ok(wave(peaks))
    })
    .await
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
    let requested = req
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok());
    let base = |status| {
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "audio/wav")
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CACHE_CONTROL, "no-store")
    };
    let unsatisfiable = |len: u64| {
        Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{len}"))
            .body(Vec::new())
    };
    match &span.grant {
        Grant::Sample { kind, t0_ms, t1_ms } => {
            let Ok(samples) = decode_span(&store, &span.meeting, *kind, *t0_ms, *t1_ms) else {
                return plain(StatusCode::NOT_FOUND);
            };
            let body = wav(&samples);
            match requested.map(|r| range(Some(r), body.len())) {
                Some(Some((a, b))) => base(StatusCode::PARTIAL_CONTENT)
                    .header(
                        header::CONTENT_RANGE,
                        format!("bytes {a}-{b}/{}", body.len()),
                    )
                    .body(body[a..=b].to_vec()),
                Some(None) => unsatisfiable(body.len() as u64),
                None => base(StatusCode::OK).body(body),
            }
        }
        Grant::Play { kinds, samples } => {
            let len = WAV_HEADER + samples * 2;
            // Without a Range header, the first chunk (as a range).
            let (a, b) = match requested {
                None => (0, len - 1),
                Some(r) => match range(Some(r), len as usize) {
                    Some((a, b)) => (a as u64, b as u64),
                    None => return unsatisfiable(len).unwrap_or_else(|_| server_error()),
                },
            };
            let b = b.min(a + MAX_CHUNK - 1);
            let Ok(body) = play_bytes(&store, &span.meeting, kinds, *samples, a, b) else {
                return plain(StatusCode::NOT_FOUND);
            };
            base(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, format!("bytes {a}-{b}/{len}"))
                .body(body)
        }
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
            grant: Grant::Sample {
                kind: TrackKind::Mic,
                t0_ms: 0,
                t1_ms: 1000,
            },
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

    /// A meeting with a 6 s mic tone whose pages around 3–4 s are missing
    /// (a discard) and a 6 s quieter system tone.
    fn two_tracks() -> (tempfile::TempDir, Arc<Store>, String) {
        use ghi_audio::Track;
        use ghi_audio::encoder::{EncoderConfig, TrackEncoder};
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::open(
                tmp.path(),
                Arc::new(ghi_store::keys::MemoryKeyStore::default()),
                ghi_store::keys::Protection::default(),
            )
            .unwrap(),
        );
        let m = store
            .create_meeting(ghi_store::store::NewMeeting::default())
            .unwrap()
            .gid;
        let tone = |amp: f32, hz: f32| -> Vec<f32> {
            (0..RATE as usize * 6)
                .map(|i| amp * (i as f32 * hz * std::f32::consts::TAU / RATE as f32).sin())
                .collect()
        };
        for (kind, track, amp, hole) in [
            (TrackKind::Mic, Track::Mic, 0.5, true),
            (TrackKind::System, Track::System, 0.2, false),
        ] {
            let (mut enc, head) = TrackEncoder::new(track, 7, &EncoderConfig::default()).unwrap();
            let mut pages: Vec<Vec<u8>> = Vec::new();
            let mut emit = |p: &[u8]| {
                pages.push(p.to_vec());
                Ok(())
            };
            enc.push(&tone(amp, 440.0), &mut emit).unwrap();
            enc.finish(&mut emit).unwrap();
            let mut w = store.open_track(&m, kind).unwrap();
            w.append(&head).unwrap();
            for p in pages {
                let end = page_end_granule(&p).unwrap_or(0) / GRANULE_PER_SAMPLE;
                if hole && end > RATE * 3 && end <= RATE * 4 {
                    continue;
                }
                w.append(&p).unwrap();
            }
            store.finish_track(&m, kind, w).unwrap();
        }
        store.finish_meeting(&m, 6_000).unwrap();
        (tmp, store, m)
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn the_whole_meeting_plays_as_one_wav_with_holes_as_silence() {
        let (_tmp, store, m) = two_tracks();
        let both = [TrackKind::Mic, TrackKind::System];
        let mic = [TrackKind::Mic];
        let at = |s: f32| (s * RATE as f32) as u64;
        // Positions are true on both sides of the hole.
        let before = decode_mix(&store, &m, &mic, at(1.0), at(1.5)).unwrap();
        let hole = decode_mix(&store, &m, &mic, at(3.3), at(3.7)).unwrap();
        let after = decode_mix(&store, &m, &mic, at(5.0), at(5.5)).unwrap();
        assert_eq!(before.len(), at(0.5) as usize);
        assert!(rms(&before) > 0.25 && rms(&after) > 0.25, "the tone");
        assert!(rms(&hole) < 0.01, "the discarded span is silent");
        // Both tracks mixed (same phase), with headroom: 0.7 × (0.5 + 0.2).
        let mixed = decode_mix(&store, &m, &both, at(1.0), at(1.5)).unwrap();
        let sys = decode_mix(&store, &m, &[TrackKind::System], at(1.0), at(1.5)).unwrap();
        let want = HEADROOM * (rms(&before) + rms(&sys));
        assert!(
            (rms(&mixed) - want).abs() < 0.03,
            "{} vs {want}",
            rms(&mixed)
        );
        // Past the end: silence, not an error.
        assert!(rms(&decode_mix(&store, &m, &both, at(7.0), at(7.5)).unwrap()) == 0.0);

        // Bytes: header, then PCM at 2 bytes per sample; any alignment.
        let samples = at(6.0);
        let head = play_bytes(&store, &m, &both, samples, 0, 1).unwrap();
        assert_eq!(head, b"RI");
        let big = play_bytes(&store, &m, &both, samples, 0, 4_000).unwrap();
        assert_eq!(big.len(), 4_001);
        assert_eq!(
            u32::from_le_bytes(big[40..44].try_into().unwrap()) as u64,
            samples * 2
        );
        let odd = play_bytes(&store, &m, &both, samples, 45, 50).unwrap();
        assert_eq!(odd, big[45..=50]);
        let tail = play_bytes(
            &store,
            &m,
            &both,
            samples,
            44 + samples * 2 - 3,
            44 + samples * 2 - 1,
        )
        .unwrap();
        assert_eq!(tail.len(), 3);
    }

    #[test]
    fn play_tokens_answer_in_chunks_and_only_the_latest_works() {
        let (_tmp, store, m) = two_tracks();
        let t = AudioTokens::default();
        // Longer than the audio (silence after it): more than one chunk.
        let samples = 60 * RATE;
        let play = |meeting: &str| Span {
            meeting: meeting.into(),
            grant: Grant::Play {
                kinds: vec![TrackKind::Mic, TrackKind::System],
                samples,
            },
            expires: Instant::now() + TTL,
        };
        let old = t.issue(play(&m));
        let token = t.issue(play(&m));
        assert!(t.get(&old).is_none(), "one meeting plays at a time");
        let req = |range: Option<&str>| {
            let mut b = Request::builder()
                .method("GET")
                .uri(format!("ghi-audio://localhost/{token}"));
            if let Some(r) = range {
                b = b.header(header::RANGE, r);
            }
            b.body(Vec::new()).unwrap()
        };
        let len = WAV_HEADER + samples * 2;
        let r = respond(Some(store.clone()), &t, &req(Some("bytes=0-")));
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(r.body().len() as u64, MAX_CHUNK);
        assert_eq!(
            r.headers()[header::CONTENT_RANGE],
            format!("bytes 0-{}/{len}", MAX_CHUNK - 1)
        );
        // A seek near the end.
        let r = respond(
            Some(store.clone()),
            &t,
            &req(Some(&format!("bytes={}-", len - 100))),
        );
        assert_eq!(r.body().len(), 100);
        let r = respond(
            Some(store.clone()),
            &t,
            &req(Some(&format!("bytes={len}-"))),
        );
        assert_eq!(r.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        // No Range header: the first chunk, still as a range.
        let r = respond(Some(store), &t, &req(None));
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
    }

    #[test]
    fn the_waveform_follows_the_audio() {
        let (_tmp, store, m) = two_tracks();
        let w = compute_waveform(&store, &m, &[TrackKind::Mic], 6 * RATE).unwrap();
        assert_eq!(w.len(), 60, "10 per second");
        assert!(w[10] > 200, "a loud tone: {}", w[10]);
        assert!(w[35] < 40, "the hole: {}", w[35]);
    }
}
