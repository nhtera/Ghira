// SPDX-License-Identifier: Apache-2.0
//! The encoder's page sink for live sessions: Ogg Opus pages into the
//! meeting's encrypted track bundles (one page per bundle record), with the
//! timing of every page so a discard knows where to cut [RT-1].

use std::io;

use ghi_audio::encoder::PageSink;
use ghi_audio::{Marker, Track};
use ghi_store::bundle::BundleWriter;

/// Granule (48 kHz samples decoded) at the end of an Ogg page; `None` for a
/// page without one (-1) or a malformed page.
pub fn page_end_granule(page: &[u8]) -> Option<u64> {
    if page.len() < 14 || &page[..4] != b"OggS" {
        return None;
    }
    let g = i64::from_le_bytes(page[6..14].try_into().ok()?);
    u64::try_from(g).ok()
}

pub struct BundlePages {
    pub writers: [Option<BundleWriter>; 2],
    /// Per track: end granule of each bundle record (record 0 = headers, 0).
    ends: [Vec<u64>; 2],
    /// During a discard: pages written after the cut was chosen wait here
    /// until the bundles are rotated, then follow the kept pages.
    held: Option<Vec<(Track, Vec<u8>)>>,
    /// Sensitive mode: pages are dropped, never written.
    off: bool,
}

impl BundlePages {
    pub fn new(writers: [Option<BundleWriter>; 2]) -> BundlePages {
        BundlePages {
            writers,
            ends: [Vec::new(), Vec::new()],
            held: None,
            off: false,
        }
    }

    /// Sensitive mode: from now on no page is written (or held). What was
    /// written before stays in the bundles until the meeting's audio is deleted.
    pub fn stop_writing(&mut self) {
        self.off = true;
        self.held = None;
    }

    /// Records to keep on `track` so nothing at or after `t_cut_ms` remains:
    /// every page that *starts* before the cut, minus the one straddling it
    /// (≤1 s of kept audio is lost rather than keeping discarded speech).
    /// Always keeps the header record.
    pub fn keep_before(&self, track: Track, t_cut_ms: i64) -> u32 {
        let cut = (t_cut_ms.max(0) as u64) * 48;
        let ends = &self.ends[track.index()];
        // Record i covers (ends[i-1], ends[i]]; keep it if it ends by the cut.
        let keep = ends.iter().take_while(|&&e| e <= cut).count();
        keep.max(1) as u32
    }

    /// Starts holding new pages back (a discard is being applied).
    pub fn hold(&mut self) {
        self.held.get_or_insert_with(Vec::new);
    }

    /// Rotates each track to its `keep` records, then writes the held
    /// pages. A track whose rotation failed gets none of them (its old file
    /// is cut back at the next start, see `Store::complete_discard_audio`).
    /// With an empty `keep`, just stops holding (a discard that failed).
    pub fn release(&mut self, keep: &[(Track, u32)]) -> io::Result<()> {
        let mut failed: Vec<Track> = Vec::new();
        let mut result = Ok(());
        for &(t, k) in keep {
            if let Err(e) = self.rotate(t, k) {
                failed.push(t);
                result = Err(store_io(e));
            }
        }
        for (t, page) in self.held.take().unwrap_or_default() {
            if !failed.contains(&t) {
                self.write_page(t, &page)?;
            }
        }
        result
    }

    /// Nonce prefix (hex) of a track's bundle file.
    pub fn prefix_hex(&self, track: Track) -> Option<String> {
        self.writers[track.index()].as_ref().map(|w| w.prefix_hex())
    }

    /// Cuts `track` back to `keep` records (see `BundleWriter::rotate`).
    pub fn rotate(&mut self, track: Track, keep: u32) -> ghi_store::Result<()> {
        if let Some(w) = self.writers[track.index()].as_mut() {
            w.rotate(keep)?;
            self.ends[track.index()].truncate(keep as usize);
        }
        Ok(())
    }
}

/// Silences frames before a position: the discarded audio still inside the
/// pipeline when a discard is applied must not reach the bundles [RT-1].
pub struct Muted<W> {
    pub inner: W,
    until: u64,
}

impl<W> Muted<W> {
    pub fn new(inner: W) -> Muted<W> {
        Muted { inner, until: 0 }
    }

    /// Frames before `pos` (timeline samples) are written as silence.
    pub fn mute_until(&mut self, pos: u64) {
        self.until = self.until.max(pos);
    }
}

impl<W: ghi_audio::pipeline::FrameSink> ghi_audio::pipeline::FrameSink for Muted<W> {
    fn frame(&mut self, track: Track, pos: u64, samples: &[f32]) -> io::Result<()> {
        if pos < self.until {
            let silence = vec![0.0; samples.len()];
            self.inner.frame(track, pos, &silence)
        } else {
            self.inner.frame(track, pos, samples)
        }
    }

    fn marker(&mut self, marker: &Marker) -> io::Result<()> {
        self.inner.marker(marker)
    }
}

/// Floor of the level meter (digital silence reads this, not -inf).
pub const LEVEL_FLOOR_DBFS: f32 = -100.0;

/// Sums the energy of the frames the pipeline emits (raw 16 kHz tracks, before
/// echo cancellation), so the pump reads levels without a second pass over
/// the audio: 160 multiplies per track per 10 ms, on the thread that already
/// handles the frame.
pub struct Metered<W> {
    pub inner: W,
    sum_sq: [f64; 2],
    count: [u32; 2],
}

impl<W> Metered<W> {
    pub fn new(inner: W) -> Metered<W> {
        Metered {
            inner,
            sum_sq: [0.0; 2],
            count: [0; 2],
        }
    }

    /// RMS in dBFS per track (mic, system) since the last call; `None` for a
    /// track that saw no frame.
    pub fn take_dbfs(&mut self) -> [Option<f32>; 2] {
        let out = [0, 1].map(|i| {
            (self.count[i] > 0).then(|| {
                let ms = self.sum_sq[i] / f64::from(self.count[i]);
                ((10.0 * ms.log10()) as f32).max(LEVEL_FLOOR_DBFS)
            })
        });
        self.sum_sq = [0.0; 2];
        self.count = [0; 2];
        out
    }
}

impl<W: ghi_audio::pipeline::FrameSink> ghi_audio::pipeline::FrameSink for Metered<W> {
    fn frame(&mut self, track: Track, pos: u64, samples: &[f32]) -> io::Result<()> {
        let i = track.index();
        self.sum_sq[i] += samples.iter().map(|&s| f64::from(s * s)).sum::<f64>();
        self.count[i] += samples.len() as u32;
        self.inner.frame(track, pos, samples)
    }

    fn marker(&mut self, marker: &Marker) -> io::Result<()> {
        self.inner.marker(marker)
    }
}

fn store_io(e: ghi_store::StoreError) -> io::Error {
    io::Error::other(e.to_string())
}

impl PageSink for BundlePages {
    fn write_page(&mut self, track: Track, page: &[u8]) -> io::Result<()> {
        if self.off {
            return Ok(());
        }
        if let Some(held) = self.held.as_mut() {
            held.push((track, page.to_vec()));
            return Ok(());
        }
        let i = track.index();
        let Some(w) = self.writers[i].as_mut() else {
            return Ok(());
        };
        w.append(page).map_err(store_io)?;
        // A record may hold several Ogg pages (the headers); the last one's
        // granule ends it.
        let mut end = self.ends[i].last().copied().unwrap_or(0);
        let mut at = 0;
        while at + 27 <= page.len() && &page[at..at + 4] == b"OggS" {
            let segs = page[at + 26] as usize;
            if at + 27 + segs > page.len() {
                break;
            }
            let body: usize = page[at + 27..at + 27 + segs]
                .iter()
                .map(|&b| b as usize)
                .sum();
            if let Some(g) = page_end_granule(&page[at..]) {
                end = end.max(g);
            }
            at += 27 + segs + body;
        }
        self.ends[i].push(end);
        Ok(())
    }

    fn sync(&mut self, track: Track, durable: bool) -> io::Result<()> {
        if self.held.is_some() {
            return Ok(());
        }
        match self.writers[track.index()].as_ref() {
            Some(w) => w.sync(durable).map_err(store_io),
            None => Ok(()),
        }
    }

    fn marker(&mut self, _marker: &Marker) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_audio::encoder::{EncoderConfig, OpusRecorder};
    use ghi_audio::pipeline::FrameSink;
    use ghi_store::rowcrypt::Dek;

    #[test]
    fn page_timing_finds_the_cut() {
        let dir = tempfile::tempdir().unwrap();
        let k = Dek::generate();
        let w = BundleWriter::create(&dir.path().join("mic.ghb"), &k, b"t").unwrap();
        let mut rec = OpusRecorder::new(
            BundlePages::new([Some(w), None]),
            &[Track::Mic],
            EncoderConfig::default(),
        )
        .unwrap();
        // 5 s of a tone in 10 ms frames: ~5 one-second pages after the headers.
        let frame: Vec<f32> = (0..160).map(|i| (i as f32 * 0.1).sin() * 0.2).collect();
        for pos in 0..500u64 {
            rec.frame(Track::Mic, pos * 160, &frame).unwrap();
        }
        rec.flush().unwrap();
        let pages = rec.sink_mut();
        let ends = pages.ends[0].clone();
        assert_eq!(ends[0], 0, "headers");
        assert!(ends.windows(2).all(|w| w[0] <= w[1]));
        assert!(ends.len() >= 5);
        // Cut at 2.5 s: keep headers + pages ending by 2.5 s (2 full seconds).
        let keep = pages.keep_before(Track::Mic, 2_500);
        assert!(ends[keep as usize - 1] <= 2_500 * 48);
        assert!(ends[keep as usize] > 2_500 * 48);
        assert_eq!(pages.keep_before(Track::Mic, 0), 1, "headers stay");
        pages.rotate(Track::Mic, keep).unwrap();
        assert_eq!(pages.writers[0].as_ref().unwrap().page_count(), keep);
        assert_eq!(page_end_granule(b"nope"), None);
    }
}
