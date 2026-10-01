// SPDX-License-Identifier: Apache-2.0
//! What the final pass changed: for each v2 line, the v1 (live) lines it
//! overlaps in time and whether the text or the speaker differs. Drives the
//! "changed" view (doc 02 §E) and the edited-segment suggestions [RT-2].

/// One line of either version.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffLine {
    pub gid: String,
    pub speaker: Option<u32>,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// The v2 line.
    pub new: String,
    /// The v1 lines it replaces (time overlap).
    pub old: Vec<String>,
    pub text_changed: bool,
    pub speaker_changed: bool,
}

/// Folded words, so case, accents and punctuation don't count as changes.
fn norm(s: &str) -> Vec<String> {
    ghi_text::tokens(&ghi_text::fold(s))
        .into_iter()
        .map(|(_, w)| w)
        .collect()
}

/// v2 lines that differ from the v1 lines under them. `speaker_of_v1` maps a
/// v1 speaker to the v2 speaker it carried over to (`None`: not carried).
pub fn changes(
    v1: &[DiffLine],
    v2: &[DiffLine],
    speaker_of_v1: impl Fn(u32) -> Option<u32>,
) -> Vec<Change> {
    let mut out = Vec::new();
    for n in v2 {
        let under: Vec<&DiffLine> = v1
            .iter()
            .filter(|o| o.t0_ms < n.t1_ms && n.t0_ms < o.t1_ms)
            .collect();
        let old_words: Vec<String> = under.iter().flat_map(|o| norm(&o.text)).collect();
        let text_changed = old_words != norm(&n.text);
        // The speaker of the v1 line that overlaps most.
        let main = under
            .iter()
            .max_by_key(|o| o.t1_ms.min(n.t1_ms) - o.t0_ms.max(n.t0_ms))
            .and_then(|o| o.speaker);
        let speaker_changed = main.and_then(&speaker_of_v1) != n.speaker;
        if text_changed || speaker_changed {
            out.push(Change {
                new: n.gid.clone(),
                old: under.iter().map(|o| o.gid.clone()).collect(),
                text_changed,
                speaker_changed,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(gid: &str, speaker: u32, t0: i64, t1: i64, text: &str) -> DiffLine {
        DiffLine {
            gid: gid.into(),
            speaker: Some(speaker),
            t0_ms: t0,
            t1_ms: t1,
            text: text.into(),
        }
    }

    #[test]
    fn reports_text_and_speaker_changes_only() {
        let v1 = [
            line("a", 1, 0, 2_000, "Chốt lịch beta."),
            line("b", 1, 2_000, 4_000, "Ngân sách quý ba"),
            line("c", 2, 4_000, 6_000, "okay"),
        ];
        let v2 = [
            line("x", 7, 0, 2_000, "chot lich beta"),
            line("y", 7, 2_000, 4_000, "Ngân sách quý bốn"),
            line("z", 7, 4_000, 6_000, "okay"),
        ];
        // Live 1 → final 7; live 2 → final 8.
        let map = |s: u32| Some(if s == 1 { 7 } else { 8 });
        let c = changes(&v1, &v2, map);
        assert_eq!(c.len(), 2);
        assert_eq!(
            (c[0].new.as_str(), c[0].text_changed, c[0].speaker_changed),
            ("y", true, false)
        );
        assert_eq!(
            (c[1].new.as_str(), c[1].text_changed, c[1].speaker_changed),
            ("z", false, true)
        );
        assert_eq!(c[1].old, ["c"]);
    }
}
