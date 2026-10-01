// SPDX-License-Identifier: Apache-2.0
//! SpeakerTracker: maps diarization labels to the meeting's speakers.
//!
//! - Diarizer labels are per stream (`Source`: track + stream generation +
//!   label); a reset stream starts a new generation, so its labels are new
//!   speakers until the final pass carries names over.
//! - A new label is *provisional* for [`PROVISIONAL_S`]; then it becomes
//!   "Speaker N", numbered in order of arrival.
//! - Call mode: the mic track is Me (no voice matching yet; doc 05 §5).
//! - At most [`LANES`] speakers get their own lane and color; later ones go
//!   to "Others" (re-clustered by the final pass).
//! - User edits: rename, merge (future lines of the merged speaker go to the
//!   target), split (a new speaker the caller moves lines to), not a person.
//!
//! Pure state: the session persists speakers and turns [`Change`]s into events.

use std::collections::HashMap;

/// How long a new speaker stays provisional (seconds of meeting time).
pub const PROVISIONAL_S: f64 = 2.5;
/// Speakers with their own lane; the rest share "Others".
pub const LANES: usize = 8;
/// Color slots in order of arrival (design notes: s1 blue, s2 amber, s4
/// magenta, s8 brown, then s5, s3, s7, s6).
pub const COLOR_ORDER: [u8; 8] = [1, 2, 4, 8, 5, 3, 7, 6];

/// Session speaker id (1-based, stable for the meeting).
pub type SpeakerId = u32;

/// Where a diarization label comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Source {
    /// 0 = mic, 1 = system (`ghi_audio::Track::index`).
    pub track: u8,
    /// Bumped when the track's diarizer stream is reopened.
    pub generation: u32,
    /// The diarizer's 1-based label.
    pub label: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Speaker {
    pub id: SpeakerId,
    /// "Speaker N" number; 0 while provisional and for Me.
    pub number: u32,
    pub name: Option<String>,
    /// 1..=8, or 0 for the Others lane.
    pub color_slot: u8,
    pub is_me: bool,
    pub provisional: bool,
    pub not_person: bool,
    /// Lines of this speaker now belong to `merged_into`.
    pub merged_into: Option<SpeakerId>,
    /// In the shared "Others" lane.
    pub others: bool,
    first_t: f64,
}

/// What changed, for events and persistence.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Arrived(SpeakerId),
    Confirmed(SpeakerId),
    Renamed(SpeakerId),
    Merged { from: SpeakerId, into: SpeakerId },
    Split { from: SpeakerId, new: SpeakerId },
    NotAPerson(SpeakerId),
}

#[derive(Debug, Default)]
pub struct SpeakerTracker {
    speakers: Vec<Speaker>,
    sources: HashMap<Source, SpeakerId>,
    me: Option<SpeakerId>,
    numbered: u32,
    lanes_used: usize,
}

impl SpeakerTracker {
    pub fn new() -> SpeakerTracker {
        SpeakerTracker::default()
    }

    pub fn speakers(&self) -> &[Speaker] {
        &self.speakers
    }

    pub fn get(&self, id: SpeakerId) -> Option<&Speaker> {
        self.speakers.get(id.checked_sub(1)? as usize)
    }

    fn get_mut(&mut self, id: SpeakerId) -> Option<&mut Speaker> {
        self.speakers.get_mut(id.checked_sub(1)? as usize)
    }

    /// Follows merges to the speaker that owns the lines now.
    pub fn canonical(&self, mut id: SpeakerId) -> SpeakerId {
        for _ in 0..self.speakers.len() {
            match self.get(id).and_then(|s| s.merged_into) {
                Some(next) => id = next,
                None => break,
            }
        }
        id
    }

    fn add(&mut self, t: f64, is_me: bool) -> SpeakerId {
        let id = self.speakers.len() as SpeakerId + 1;
        let lane = self.lanes_used < LANES;
        let color_slot = if lane {
            self.lanes_used += 1;
            COLOR_ORDER[self.lanes_used - 1]
        } else {
            0
        };
        self.speakers.push(Speaker {
            id,
            number: 0,
            name: None,
            color_slot,
            is_me,
            provisional: !is_me,
            not_person: false,
            merged_into: None,
            others: !lane,
            first_t: t,
        });
        id
    }

    /// The speaker for a diarization label heard at `t` (creating a
    /// provisional one for a new label).
    pub fn resolve(&mut self, src: Source, t: f64, changes: &mut Vec<Change>) -> SpeakerId {
        if let Some(&id) = self.sources.get(&src) {
            return self.canonical(id);
        }
        let id = self.add(t, false);
        self.sources.insert(src, id);
        changes.push(Change::Arrived(id));
        id
    }

    /// Me (Call mode: whoever speaks into the mic track).
    pub fn me(&mut self, t: f64, changes: &mut Vec<Change>) -> SpeakerId {
        if let Some(id) = self.me {
            return self.canonical(id);
        }
        let id = self.add(t, true);
        self.me = Some(id);
        changes.push(Change::Arrived(id));
        id
    }

    /// Confirms provisional speakers older than [`PROVISIONAL_S`] at `now`.
    pub fn tick(&mut self, now: f64, changes: &mut Vec<Change>) {
        for i in 0..self.speakers.len() {
            let s = &self.speakers[i];
            if s.provisional && s.merged_into.is_none() && now - s.first_t >= PROVISIONAL_S {
                self.numbered += 1;
                let n = self.numbered;
                let s = &mut self.speakers[i];
                s.provisional = false;
                s.number = n;
                changes.push(Change::Confirmed(s.id));
            }
        }
    }

    /// The label shown for a speaker.
    pub fn label(&self, id: SpeakerId) -> String {
        let id = self.canonical(id);
        match self.get(id) {
            None => "Unknown".into(),
            Some(s) => match (&s.name, s.is_me, s.provisional) {
                (Some(n), _, _) => n.clone(),
                (None, true, _) => "Me".into(),
                (None, false, true) => "Identifying…".into(),
                (None, false, false) => format!("Speaker {}", s.number),
            },
        }
    }

    pub fn rename(&mut self, id: SpeakerId, name: &str, changes: &mut Vec<Change>) -> bool {
        let id = self.canonical(id);
        let Some(s) = self.get_mut(id) else {
            return false;
        };
        let name = name.trim();
        s.name = (!name.is_empty()).then(|| name.to_string());
        changes.push(Change::Renamed(id));
        true
    }

    /// Merges `from` into `into`; future lines of `from` go to `into`.
    pub fn merge(&mut self, from: SpeakerId, into: SpeakerId, changes: &mut Vec<Change>) -> bool {
        let (from, into) = (self.canonical(from), self.canonical(into));
        if from == into || self.get(from).is_none() || self.get(into).is_none() {
            return false;
        }
        let was_me = self.get(from).is_some_and(|s| s.is_me);
        if let Some(s) = self.get_mut(from) {
            s.merged_into = Some(into);
        }
        if was_me {
            if let Some(s) = self.get_mut(into) {
                s.is_me = true;
            }
            self.me = Some(into);
        }
        changes.push(Change::Merged { from, into });
        true
    }

    /// A new speaker split off `from`; the caller moves the chosen lines to it.
    pub fn split(
        &mut self,
        from: SpeakerId,
        t: f64,
        changes: &mut Vec<Change>,
    ) -> Option<SpeakerId> {
        let from = self.canonical(from);
        self.get(from)?;
        let new = self.add(t, false);
        // Split-off speakers are confirmed at once: the user made them.
        self.numbered += 1;
        let n = self.numbered;
        if let Some(s) = self.get_mut(new) {
            s.provisional = false;
            s.number = n;
        }
        changes.push(Change::Split { from, new });
        Some(new)
    }

    pub fn set_not_person(&mut self, id: SpeakerId, changes: &mut Vec<Change>) -> bool {
        let id = self.canonical(id);
        let Some(s) = self.get_mut(id) else {
            return false;
        };
        s.not_person = true;
        changes.push(Change::NotAPerson(id));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(track: u8, label: u32) -> Source {
        Source {
            track,
            generation: 0,
            label,
        }
    }

    #[test]
    fn arrival_order_provisional_then_numbered_with_colors() {
        let mut t = SpeakerTracker::new();
        let mut ch = Vec::new();
        let me = t.me(0.0, &mut ch);
        let a = t.resolve(src(1, 1), 1.0, &mut ch);
        let b = t.resolve(src(1, 2), 2.0, &mut ch);
        assert_eq!(
            t.resolve(src(1, 1), 3.0, &mut ch),
            a,
            "same label, same speaker"
        );
        assert_eq!(t.label(me), "Me");
        assert_eq!(t.label(a), "Identifying…");
        t.tick(3.6, &mut ch);
        assert_eq!(t.label(a), "Speaker 1");
        assert_eq!(t.label(b), "Identifying…", "only 1.6 s old");
        t.tick(4.5, &mut ch);
        assert_eq!(t.label(b), "Speaker 2");
        let colors: Vec<u8> = t.speakers().iter().map(|s| s.color_slot).collect();
        assert_eq!(colors, [1, 2, 4]);
        assert_eq!(
            ch,
            [
                Change::Arrived(me),
                Change::Arrived(a),
                Change::Arrived(b),
                Change::Confirmed(a),
                Change::Confirmed(b)
            ]
        );
    }

    #[test]
    fn the_ninth_speaker_goes_to_others() {
        let mut t = SpeakerTracker::new();
        let mut ch = Vec::new();
        for label in 1..=9 {
            t.resolve(src(1, label), label as f64, &mut ch);
        }
        let ninth = t.get(9).unwrap();
        assert!(ninth.others && ninth.color_slot == 0);
        assert_eq!(t.get(8).unwrap().color_slot, 6);
    }

    #[test]
    fn rename_merge_split_and_not_a_person() {
        let mut t = SpeakerTracker::new();
        let mut ch = Vec::new();
        let me = t.me(0.0, &mut ch);
        let a = t.resolve(src(1, 1), 1.0, &mut ch);
        let b = t.resolve(src(1, 2), 1.5, &mut ch);
        t.tick(10.0, &mut ch);
        assert!(t.rename(a, "  Lan  ", &mut ch));
        assert_eq!(t.label(a), "Lan");
        // b was really Lan: later lines of label 2 go to Lan.
        assert!(t.merge(b, a, &mut ch));
        assert_eq!(t.resolve(src(1, 2), 12.0, &mut ch), a);
        assert!(!t.merge(a, a, &mut ch), "no self-merge");
        // Me merged into a diarized speaker keeps Me.
        assert!(t.merge(me, a, &mut ch));
        assert!(t.get(a).unwrap().is_me);
        assert_eq!(t.me(13.0, &mut ch), a);
        let new = t.split(a, 14.0, &mut ch).unwrap();
        assert_eq!(t.label(new), "Speaker 3");
        assert!(t.set_not_person(new, &mut ch));
        assert!(t.get(new).unwrap().not_person);
        assert!(t.rename(a, "", &mut ch));
        assert_eq!(t.label(a), "Me", "an empty name clears it");
    }

    #[test]
    fn a_new_stream_generation_is_a_new_speaker() {
        let mut t = SpeakerTracker::new();
        let mut ch = Vec::new();
        let a = t.resolve(src(1, 1), 0.0, &mut ch);
        let b = t.resolve(
            Source {
                track: 1,
                generation: 1,
                label: 1,
            },
            5.0,
            &mut ch,
        );
        assert_ne!(a, b);
    }
}
