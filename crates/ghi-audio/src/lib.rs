// SPDX-License-Identifier: Apache-2.0
//! PCM ring buffers, resampling, echo cancellation and the Ogg/Opus writer.
//!
//! The platform-independent capture core. A platform layer (macOS: the Swift
//! `GhiAudioMac` package) pushes mono `f32` blocks into one [`ring`] per
//! [`Track`] from its realtime callback. A [`pipeline::Pipeline`] thread drains
//! the rings, resamples each track onto a shared 16 kHz timeline
//! ([`resample`]), runs echo cancellation ([`aec`]) and fans out to the
//! writer ([`encoder`], [`pipeline::FrameSink`]) and to ASR
//! ([`pipeline::AsrConsumer`]). [`detect`] holds the meeting auto-detect policy.
//!
//! Library code never prints: the release profile aborts on panic and the
//! desktop app has no stdout. Problems surface as [`CaptureEvent`]s.

pub mod aec;
pub mod detect;
pub mod encoder;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod pipeline;
pub mod resample;
pub mod ring;

/// Sample rate of the common timeline, in Hz.
pub const SAMPLE_RATE: u32 = 16_000;
/// Samples per pipeline frame (10 ms, the AEC block size).
pub const FRAME_SAMPLES: usize = 160;

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// One of the two independently captured tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Track {
    /// The microphone.
    Mic,
    /// Everything the Mac plays (the far end of a call).
    System,
}

impl Track {
    pub const ALL: [Track; 2] = [Track::Mic, Track::System];

    /// Array index: `Mic` = 0, `System` = 1 (matches `GHI_MAC_TRACK_*`).
    pub const fn index(self) -> usize {
        match self {
            Track::Mic => 0,
            Track::System => 1,
        }
    }

    pub const fn from_index(i: u32) -> Option<Track> {
        match i {
            0 => Some(Track::Mic),
            1 => Some(Track::System),
            _ => None,
        }
    }

    /// Lowercase name, used in file names (`<id>.mic.wav`).
    pub const fn name(self) -> &'static str {
        match self {
            Track::Mic => "mic",
            Track::System => "system",
        }
    }
}

/// Where the Mac's output currently goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Route {
    #[default]
    Unknown,
    Speakers,
    Headphones,
    Bluetooth,
    External,
}

impl Route {
    /// Decodes `GHI_MAC_ROUTE_*` (the low byte of the route event code).
    pub const fn from_code(code: u32) -> Route {
        match code & 0xff {
            1 => Route::Speakers,
            2 => Route::Headphones,
            3 => Route::Bluetooth,
            4 => Route::External,
            _ => Route::Unknown,
        }
    }

    /// Whether the far end can leak into the microphone, so echo cancellation
    /// should run. Unknown is treated as "yes": cancelling nothing is cheap,
    /// double-counted remote speech is not.
    pub const fn wants_aec(self) -> bool {
        matches!(self, Route::Speakers | Route::External | Route::Unknown)
    }
}

/// A point on the timeline the writer must preserve. `pos` is in samples of
/// the 16 kHz timeline (see [`SAMPLE_RATE`]); pauses do not advance it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marker {
    pub pos: u64,
    pub kind: MarkerKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarkerKind {
    Pause,
    Resume,
    /// The user discarded the last `last_s` seconds (capture layer only;
    /// phase 8 owns the transaction over transcript and notes).
    Discard {
        last_s: f32,
    },
    /// Silence was inserted on `track` for `from..to` (sleep, device rebuild,
    /// dropped blocks). Both are timeline samples; `Marker::pos == from`.
    Gap {
        track: Track,
        from: u64,
        to: u64,
    },
    /// From here the ASR mic signal is echo-cancelled; the stored mic track
    /// stays raw so the final pass can redo it.
    AecOn,
    AecOff,
    /// The Mac went to sleep: like a pause, the timeline stops here.
    Sleep,
    /// The Mac woke after `slept_s` seconds of wall time (0 if unknown).
    /// The timeline continues at the sleep position; no silence is inserted.
    Wake {
        slept_s: f32,
    },
}

/// Typed capture events for the UI (phase 10) and the CLI.
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureEvent {
    TrackStarted {
        track: Track,
        device: String,
    },
    /// `input_bluetooth_hfp`: the default input is a Bluetooth headset, whose
    /// mic drops the whole link to 16 kHz call quality.
    RouteChanged {
        route: Route,
        input_bluetooth_hfp: bool,
    },
    MicDeviceChanged {
        device: String,
    },
    SystemRestarted,
    TrackLost {
        track: Track,
    },
    Sleep,
    Wake,
    /// The system track has been digital silence for `silent_s` seconds while
    /// system capture is on (a denied or broken tap yields zeros). Raised once
    /// per silent stretch.
    SilentSystemTrack {
        silent_s: f32,
    },
    DiskLow {
        free_bytes: u64,
    },
    DiskFull,
    /// A ring overflowed and `dropped` samples of `track` were lost.
    Overrun {
        track: Track,
        dropped: u64,
    },
    /// The ASR ring overflowed; `frames` 10 ms frames were skipped for ASR
    /// (the stored audio is complete).
    AsrSkipped {
        frames: u64,
    },
    Error {
        code: i64,
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_codes_and_aec_policy() {
        assert_eq!(Route::from_code(1), Route::Speakers);
        assert_eq!(Route::from_code(0x103), Route::Bluetooth);
        assert_eq!(Route::from_code(99), Route::Unknown);
        assert!(Route::Speakers.wants_aec());
        assert!(Route::External.wants_aec());
        assert!(Route::Unknown.wants_aec());
        assert!(!Route::Headphones.wants_aec());
        assert!(!Route::Bluetooth.wants_aec());
    }

    #[test]
    fn track_indices() {
        for t in Track::ALL {
            assert_eq!(Track::from_index(t.index() as u32), Some(t));
        }
        assert_eq!(Track::from_index(2), None);
    }
}
