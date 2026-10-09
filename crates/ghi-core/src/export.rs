// SPDX-License-Identifier: Apache-2.0
//! Meeting exports: Markdown, plain text, SRT, VTT, Word (.docx) and an
//! Obsidian note. Pure rendering from the store, no clocks: dates are UTC
//! (`YYYY-MM-DD HH:MM`) from the meeting's `started_at`, so the same meeting
//! always renders the same bytes. Transcript and note text is untrusted
//! (RT-6): Markdown text is escaped so it stays text, and no raw HTML is ever
//! written (VTT cue text is entity-escaped).

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Cursor, ErrorKind, Write};
use std::path::Path;

use docx_rs::{BreakType, Docx, Paragraph, Run, Style, StyleType};
use ghi_store::store::{Meeting, Provenance, Segment, Store};

use crate::notes_job::{ENHANCED_PREFIX, speaker_label};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Markdown,
    Text,
    Srt,
    Vtt,
    Docx,
}

/// Language of the fixed headings (the app's UI language).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Vi,
}

#[derive(Debug, Clone, Copy)]
pub struct ExportOptions {
    pub include_notes: bool,
    pub include_transcript: bool,
    pub ui_lang: Lang,
}

/// The file extension (no dot) for `f`.
pub fn extension(f: Format) -> &'static str {
    match f {
        Format::Markdown => "md",
        Format::Text => "txt",
        Format::Srt => "srt",
        Format::Vtt => "vtt",
        Format::Docx => "docx",
    }
}

/// Fixed headings and labels, per UI language.
struct Strings {
    summary: &'static str,
    decisions: &'static str,
    proposed: &'static str,
    answers: &'static str,
    actions: &'static str,
    questions: &'static str,
    quotes: &'static str,
    topics: &'static str,
    my_notes: &'static str,
    transcript: &'static str,
    date: &'static str,
    duration: &'static str,
    participants: &'static str,
    due: &'static str,
    note: &'static str,
    decision: &'static str,
    action: &'static str,
    question: &'static str,
}

const EN: Strings = Strings {
    summary: "Summary",
    decisions: "Decisions",
    proposed: "Proposed",
    answers: "Saved answers",
    actions: "Action items",
    questions: "Open questions",
    quotes: "Key quotes",
    topics: "Topics",
    my_notes: "My notes",
    transcript: "Transcript",
    date: "Date",
    duration: "Duration",
    participants: "Participants",
    due: "due",
    note: "Note",
    decision: "Decision",
    action: "Action",
    question: "Question",
};

const VI: Strings = Strings {
    summary: "Tóm tắt",
    decisions: "Quyết định",
    proposed: "Đề xuất",
    answers: "Câu trả lời đã lưu",
    actions: "Việc cần làm",
    questions: "Câu hỏi mở",
    quotes: "Trích dẫn chính",
    topics: "Chủ đề",
    my_notes: "Ghi chú của tôi",
    transcript: "Bản ghi",
    date: "Ngày",
    duration: "Thời lượng",
    participants: "Người tham gia",
    due: "hạn",
    note: "Ghi chú",
    decision: "Quyết định",
    action: "Việc",
    question: "Câu hỏi",
};

fn strings(l: Lang) -> &'static Strings {
    match l {
        Lang::En => &EN,
        Lang::Vi => &VI,
    }
}

fn store_err(e: ghi_store::StoreError) -> String {
    e.to_string()
}

/// Renders `meeting` as `format`. Subtitle formats ignore `opts` (they are the
/// transcript, nothing else).
pub fn render(
    store: &Store,
    meeting: &str,
    format: Format,
    opts: &ExportOptions,
) -> Result<Vec<u8>, String> {
    match format {
        Format::Srt | Format::Vtt => {
            let cues = Cues::load(store, meeting)?;
            Ok(if format == Format::Srt {
                cues.srt()
            } else {
                cues.vtt()
            }
            .into_bytes())
        }
        Format::Markdown => Ok(Model::load(store, meeting, opts)?.markdown().into_bytes()),
        Format::Text => Ok(Model::load(store, meeting, opts)?.text().into_bytes()),
        Format::Docx => Model::load(store, meeting, opts)?.docx(),
    }
}

/// A safe file stem from the title and date ("2026-10-02 Weekly sync"): no
/// path separators or reserved characters, no leading dots, at most 80
/// characters, NFC; "Meeting" when the title is empty.
pub fn file_stem(m: &Meeting) -> String {
    let title = clean_name(&m.title);
    let title = if title.is_empty() {
        "Meeting".to_string()
    } else {
        title
    };
    let stem = clean_name(&format!("{} {title}", date_of(m.started_at)));
    let cut: String = stem.chars().take(80).collect();
    let cut = clean_name(&cut);
    if cut.is_empty() {
        "Meeting".into()
    } else {
        cut
    }
}

/// A tag name as Obsidian accepts it: letters (any script), digits, `_`, `-`
/// and `/`, no spaces ("Q4 plan" becomes "Q4-plan"); `#`, commas and every
/// other character are dropped, so the value is safe in a YAML list. `None`
/// when nothing is left or only digits are (Obsidian rejects those).
fn obsidian_tag(name: &str) -> Option<String> {
    let mut out = String::new();
    for c in name.trim().chars() {
        let c = if c.is_whitespace() { '-' } else { c };
        if c.is_alphanumeric() || matches!(c, '_' | '-' | '/') {
            if c == '-' && out.ends_with('-') {
                continue;
            }
            out.push(c);
        }
    }
    let out = out.trim_matches(|c| c == '-' || c == '/').to_string();
    (!out.is_empty() && !out.chars().all(|c| c.is_ascii_digit())).then_some(out)
}

/// Writes the Obsidian note `<stem>.md` into `dir`, never over a file that is
/// there (" (2)", " (3)" … is appended). Returns the file name written.
pub fn write_obsidian(
    store: &Store,
    meeting: &str,
    dir: &Path,
    opts: &ExportOptions,
) -> Result<String, String> {
    let m = store.get_meeting(meeting).map_err(store_err)?;
    let model = Model::load(store, meeting, opts)?;
    let body = format!("{}{}", model.front_matter(), model.markdown());
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    write_new(dir, &file_stem(&m), "md", body.as_bytes())
}

/// Writes `bytes` to `dir/<stem>.<ext>`, never over an existing file (then
/// "<stem> (2).<ext>", …). Returns the file name written.
pub fn write_new(dir: &Path, stem: &str, ext: &str, bytes: &[u8]) -> Result<String, String> {
    for n in 1u32..10_000 {
        let name = if n == 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(&name))
        {
            Ok(mut f) => {
                f.write_all(bytes).map_err(|e| e.to_string())?;
                return Ok(name);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("too many files with this name".into())
}

// ---- names, dates, clocks ----

/// Replaces reserved/control characters with spaces, collapses whitespace,
/// trims spaces and dots at both ends, NFC.
fn clean_name(s: &str) -> String {
    let spaced: String = s
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    ghi_text::nfc(joined.trim_matches(|c: char| c == '.' || c.is_whitespace()))
}

/// `(year, month, day, hour, minute)` in UTC from unix ms.
fn civil(ms: i64) -> (i64, i64, i64, i64, i64) {
    let days = ms.div_euclid(86_400_000);
    let mins = ms.rem_euclid(86_400_000) / 60_000;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, mins / 60, mins % 60)
}

pub(crate) fn date_of(ms: i64) -> String {
    let (y, m, d, _, _) = civil(ms);
    format!("{y:04}-{m:02}-{d:02}")
}

fn datetime_of(ms: i64) -> String {
    let (_, _, _, h, min) = civil(ms);
    format!("{} {h:02}:{min:02}", date_of(ms))
}

/// `mm:ss`, or `h:mm:ss` from an hour on.
fn clock(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// `HH:MM:SS<sep>mmm` for subtitle cues.
fn cue_time(ms: i64, sep: char) -> String {
    let ms = ms.max(0);
    format!(
        "{:02}:{:02}:{:02}{sep}{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

// ---- subtitles ----

struct Cue {
    t0: i64,
    t1: i64,
    speaker: Option<String>,
    text: String,
}

struct Cues(Vec<Cue>);

impl Cues {
    fn load(store: &Store, meeting: &str) -> Result<Cues, String> {
        let names = speaker_names(store, meeting)?;
        let segs = store.segments(meeting).map_err(store_err)?;
        Ok(Cues(
            segs.into_iter()
                .filter(|s| !s.text.trim().is_empty())
                .map(|s| Cue {
                    t0: s.t0_ms,
                    t1: s.t1_ms.max(s.t0_ms),
                    speaker: s.speaker_gid.as_ref().map(|g| name_of(&names, g)),
                    text: s.text,
                })
                .collect(),
        ))
    }

    /// Cue text lines (no blank line: it would end the cue), `esc` applied to
    /// the speaker and the text.
    fn body(c: &Cue, esc: impl Fn(&str) -> String) -> String {
        let text = c
            .text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(&esc)
            .collect::<Vec<_>>()
            .join("\n");
        match &c.speaker {
            Some(s) => format!("{}: {text}", esc(s)),
            None => text,
        }
    }

    fn srt(&self) -> String {
        let mut out = String::new();
        for (i, c) in self.0.iter().enumerate() {
            out += &format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                cue_time(c.t0, ','),
                cue_time(c.t1, ','),
                Self::body(c, |s| s.replace("-->", "->"))
            );
        }
        out
    }

    fn vtt(&self) -> String {
        let mut out = String::from("WEBVTT\n\n");
        for c in &self.0 {
            out += &format!(
                "{} --> {}\n{}\n\n",
                cue_time(c.t0, '.'),
                cue_time(c.t1, '.'),
                Self::body(c, vtt_escape)
            );
        }
        out
    }
}

/// `&`, `<`, `>` as entities: no tags, and `-->` cannot survive.
fn vtt_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---- the document model ----

fn speaker_names(store: &Store, meeting: &str) -> Result<HashMap<String, String>, String> {
    Ok(store
        .speakers(meeting)
        .map_err(store_err)?
        .iter()
        .map(|s| (s.gid.clone(), speaker_label(s)))
        .collect())
}

fn name_of(names: &HashMap<String, String>, gid: &str) -> String {
    names.get(gid).cloned().unwrap_or_else(|| "Speaker".into())
}

enum Item {
    /// `sub`: the AI's expansion of a user's note, as sub-bullets.
    Bullet { text: String, sub: Vec<String> },
    Action {
        text: String,
        owner: Option<String>,
        due: Option<String>,
        done: bool,
    },
}

struct Section {
    heading: String,
    items: Vec<Item>,
}

/// Consecutive segments of one speaker, merged.
struct Turn {
    speaker: String,
    t0_ms: i64,
    text: String,
}

struct Model {
    title: String,
    started_at: i64,
    duration_ms: i64,
    participants: Vec<String>,
    /// The meeting's tags, by their names (front matter of the Obsidian note).
    tags: Vec<String>,
    sections: Vec<Section>,
    transcript: Vec<Turn>,
    s: &'static Strings,
}

impl Model {
    fn load(store: &Store, meeting: &str, opts: &ExportOptions) -> Result<Model, String> {
        let s = strings(opts.ui_lang);
        let m = store.get_meeting(meeting).map_err(store_err)?;
        let names = speaker_names(store, meeting)?;
        let segs = store.segments(meeting).map_err(store_err)?;
        let transcript = turns(&segs, &names);

        let mut participants: Vec<String> = Vec::new();
        for t in &transcript {
            if !participants.contains(&t.speaker) {
                participants.push(t.speaker.clone());
            }
        }

        let sections = if opts.include_notes {
            note_sections(store, meeting, &m, &names, opts.ui_lang)?
        } else {
            Vec::new()
        };
        let tags = store
            .meeting_tags(&[meeting.to_string()])
            .map_err(store_err)?
            .remove(meeting)
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.name)
            .collect();
        Ok(Model {
            title: m.title.trim().to_string(),
            started_at: m.started_at,
            duration_ms: m.duration_ms,
            participants,
            tags,
            sections,
            transcript: if opts.include_transcript {
                transcript
            } else {
                Vec::new()
            },
            s,
        })
    }

    /// `(label, value)` pairs under the title.
    fn meta(&self) -> Vec<(&'static str, String)> {
        let mut v = vec![
            (self.s.date, format!("{} UTC", datetime_of(self.started_at))),
            (self.s.duration, clock(self.duration_ms)),
        ];
        if !self.participants.is_empty() {
            v.push((self.s.participants, self.participants.join(", ")));
        }
        v
    }

    fn front_matter(&self) -> String {
        let q = |s: &str| {
            let mut o = String::from("\"");
            for c in s.chars() {
                match c {
                    '"' => o += "\\\"",
                    '\\' => o += "\\\\",
                    '\n' => o += "\\n",
                    '\r' => o += "\\r",
                    '\t' => o += "\\t",
                    c if c.is_control() => o += &format!("\\u{:04x}", c as u32),
                    c => o.push(c),
                }
            }
            o + "\""
        };
        let people: Vec<String> = self.participants.iter().map(|p| q(p)).collect();
        let (y, mo, d, h, mi) = civil(self.started_at);
        // `tags:` is what Obsidian indexes (safe names); `ghira_tags:` keeps the names as typed.
        let mut tags = vec!["ghira".to_string()];
        for t in self.tags.iter().filter_map(|t| obsidian_tag(t)) {
            if !tags.iter().any(|x| x.to_lowercase() == t.to_lowercase()) {
                tags.push(t);
            }
        }
        let originals = if self.tags.is_empty() {
            String::new()
        } else {
            let names: Vec<String> = self.tags.iter().map(|t| q(t)).collect();
            format!("ghira_tags: [{}]\n", names.join(", "))
        };
        format!(
            "---\ntitle: {}\ndate: {y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}\nduration: {}\nparticipants: [{}]\ntags: [{}]\n{originals}---\n\n",
            q(&self.title),
            q(&clock(self.duration_ms)),
            people.join(", "),
            tags.join(", ")
        )
    }

    fn markdown(&self) -> String {
        let mut out = format!("# {}\n\n", md_escape(&self.title));
        let meta: Vec<String> = self
            .meta()
            .into_iter()
            .map(|(k, v)| format!("**{k}:** {}", md_escape(&v)))
            .collect();
        out += &meta.join(" · ");
        out += "\n";
        for sec in &self.sections {
            out += &format!("\n## {}\n\n", md_escape(&sec.heading));
            for it in &sec.items {
                out += &indent_after_first(&self.item_line(it, true), "  ");
                out.push('\n');
                for sub in Self::item_subs(it) {
                    out += &format!("  - {}\n", md_escape(sub));
                }
            }
        }
        if !self.transcript.is_empty() {
            out += &format!("\n## {}\n", self.s.transcript);
            for t in &self.transcript {
                out += &format!(
                    "\n**{}** [{}]\n\n{}\n",
                    md_escape(&t.speaker),
                    clock(t.t0_ms),
                    md_escape(&t.text)
                );
            }
        }
        out
    }

    fn text(&self) -> String {
        let mut out = format!("{}\n\n", self.title);
        for (k, v) in self.meta() {
            out += &format!("{k}: {v}\n");
        }
        for sec in &self.sections {
            out += &format!("\n{}\n\n", sec.heading);
            for it in &sec.items {
                out += &indent_after_first(&self.item_line(it, false), "  ");
                out.push('\n');
                for sub in Self::item_subs(it) {
                    out += &format!("    • {sub}\n");
                }
            }
        }
        if !self.transcript.is_empty() {
            out += &format!("\n{}\n", self.s.transcript);
            for t in &self.transcript {
                out += &format!("\n{} [{}]\n{}\n", t.speaker, clock(t.t0_ms), t.text);
            }
        }
        out
    }

    /// One bullet or action item as a line (continuation lines follow `\n`).
    /// The sub-bullet texts of `it` (empty for most items).
    fn item_subs(it: &Item) -> &[String] {
        match it {
            Item::Bullet { sub, .. } => sub,
            Item::Action { .. } => &[],
        }
    }

    fn item_line(&self, it: &Item, md: bool) -> String {
        let esc = |s: &str| if md { md_escape(s) } else { s.to_string() };
        match it {
            Item::Bullet { text, .. } => format!("{} {}", if md { "-" } else { "•" }, esc(text)),
            Item::Action {
                text,
                owner,
                due,
                done,
            } => {
                let mark = match (md, done) {
                    (true, false) => "- [ ]",
                    (true, true) => "- [x]",
                    (false, false) => "☐",
                    (false, true) => "☑",
                };
                let mut l = format!("{mark} {}", esc(text));
                if let Some(o) = owner {
                    l += &format!(" — {}", esc(o));
                }
                if let Some(d) = due {
                    l += &format!(" ({} {})", self.s.due, esc(d));
                }
                l
            }
        }
    }

    fn docx(&self) -> Result<Vec<u8>, String> {
        let style = |id: &str, size: usize| {
            Style::new(id, StyleType::Paragraph)
                .name(id)
                .bold()
                .size(size)
        };
        let mut doc = Docx::new()
            .add_style(style("Title", 40))
            .add_style(style("Heading1", 30))
            .add_style(style("Heading2", 26))
            .add_paragraph(
                Paragraph::new()
                    .style("Title")
                    .add_run(lines_run(&self.title)),
            );
        for (k, v) in self.meta() {
            doc = doc.add_paragraph(
                Paragraph::new()
                    .add_run(Run::new().add_text(format!("{k}: ")).bold())
                    .add_run(Run::new().add_text(v)),
            );
        }
        for sec in &self.sections {
            doc = doc.add_paragraph(
                Paragraph::new()
                    .style("Heading1")
                    .add_run(Run::new().add_text(&sec.heading)),
            );
            for it in &sec.items {
                let line = self.item_line(it, false);
                doc = doc.add_paragraph(Paragraph::new().add_run(lines_run(&line)));
                for sub in Self::item_subs(it) {
                    doc = doc.add_paragraph(
                        Paragraph::new()
                            .indent(Some(720), None, None, None)
                            .add_run(Run::new().add_text(format!("• {sub}"))),
                    );
                }
            }
        }
        if !self.transcript.is_empty() {
            doc = doc.add_paragraph(
                Paragraph::new()
                    .style("Heading1")
                    .add_run(Run::new().add_text(self.s.transcript)),
            );
            for t in &self.transcript {
                doc = doc.add_paragraph(
                    Paragraph::new()
                        .add_run(
                            Run::new()
                                .add_text(format!("{} [{}]", t.speaker, clock(t.t0_ms)))
                                .bold(),
                        )
                        .add_run(lines_run(&format!("  {}", t.text))),
                );
            }
        }
        let mut buf = Cursor::new(Vec::new());
        doc.build().pack(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf.into_inner())
    }
}

/// A run with `text`, a line break per newline.
fn lines_run(text: &str) -> Run {
    let mut run = Run::new();
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            run = run.add_break(BreakType::TextWrapping);
        }
        run = run.add_text(line);
    }
    run
}

fn indent_after_first(s: &str, pad: &str) -> String {
    s.lines()
        .enumerate()
        .map(|(i, l)| match (i, l.is_empty()) {
            (0, _) | (_, true) => l.to_string(),
            _ => format!("{pad}{l}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Merges consecutive segments of one speaker into turns.
fn turns(segs: &[Segment], names: &HashMap<String, String>) -> Vec<Turn> {
    let mut out: Vec<Turn> = Vec::new();
    let mut last: Option<&Option<String>> = None;
    for s in segs.iter().filter(|s| !s.text.trim().is_empty()) {
        let text = s.text.trim();
        match (&mut out.last_mut(), last) {
            (Some(t), Some(prev)) if *prev == s.speaker_gid => {
                t.text.push(' ');
                t.text.push_str(text);
            }
            _ => out.push(Turn {
                speaker: s
                    .speaker_gid
                    .as_deref()
                    .map_or_else(|| "Speaker".into(), |g| name_of(names, g)),
                t0_ms: s.t0_ms,
                text: text.to_string(),
            }),
        }
        last = Some(&s.speaker_gid);
    }
    out
}

fn note_sections(
    store: &Store,
    meeting: &str,
    m: &Meeting,
    names: &HashMap<String, String>,
    lang: Lang,
) -> Result<Vec<Section>, String> {
    let s = strings(lang);
    let blocks = store.note_blocks(meeting).map_err(store_err)?;
    let actions = store.action_items(meeting).map_err(store_err)?;
    let ai = |kind: &str| -> Vec<Item> {
        blocks
            .iter()
            .filter(|b| b.provenance != Provenance::User && b.kind == kind)
            .map(|b| Item::Bullet {
                text: b.body.clone(),
                sub: vec![],
            })
            .collect()
    };

    // Answers saved from Ask: "Q: ...\nA: ..." as a question with its answer under it.
    let answers: Vec<Item> = blocks
        .iter()
        .filter(|b| b.kind == "answer")
        .map(|b| {
            let squash = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
            match b.body.split_once("\nA: ") {
                Some((q, a)) => Item::Bullet {
                    text: squash(q),
                    sub: vec![format!("A: {}", squash(a))],
                },
                None => Item::Bullet {
                    text: squash(&b.body),
                    sub: vec![],
                },
            }
        })
        .collect();

    let mut sections = Vec::new();
    let mut push = |heading: String, items: Vec<Item>| {
        if !items.is_empty() {
            sections.push(Section { heading, items });
        }
    };
    push(s.summary.into(), ai("tldr"));

    // Template sections in template order, then any other `section:<id>`.
    let template = m
        .template
        .as_deref()
        .and_then(|id| ghi_llm::template::builtin(id).ok());
    let mut ids: Vec<(String, String)> = template
        .iter()
        .flat_map(|t| &t.sections)
        .map(|sec| {
            let title = match lang {
                Lang::En => &sec.title_en,
                Lang::Vi => &sec.title_vi,
            };
            (sec.id.clone(), title.clone())
        })
        .collect();
    for b in &blocks {
        if let Some(id) = b.kind.strip_prefix("section:")
            && !ids.iter().any(|(i, _)| i == id)
        {
            ids.push((id.to_string(), id.to_string()));
        }
    }
    for (id, title) in ids {
        push(title, ai(&format!("section:{id}")));
    }

    push(s.decisions.into(), ai("decision"));
    push(s.proposed.into(), ai("proposal"));
    let acts = actions
        .iter()
        .filter(|a| a.provenance != Provenance::User)
        .map(|a| action_item(a, names))
        .collect();
    push(s.actions.into(), acts);
    push(s.questions.into(), ai("question"));
    push(s.quotes.into(), ai("quote"));
    push(s.topics.into(), ai("topic"));
    push(s.answers.into(), answers);

    // What the user wrote, in order, each kind tagged.
    let mut mine: Vec<Item> = blocks
        .iter()
        .filter(|b| b.provenance == Provenance::User)
        .map(|b| {
            let tag = match b.kind.as_str() {
                "decision" => Some(s.decision),
                "action" => Some(s.action),
                "question" => Some(s.question),
                "note" => None,
                _ => Some(s.note),
            };
            let sub_kind = format!("{ENHANCED_PREFIX}{}", b.gid);
            Item::Bullet {
                text: match tag {
                    Some(t) => format!("{t}: {}", b.body),
                    None => b.body.clone(),
                },
                // Empty bodies are "not found" markers.
                sub: blocks
                    .iter()
                    .filter(|e| e.kind == sub_kind && !e.body.trim().is_empty())
                    .map(|e| e.body.split_whitespace().collect::<Vec<_>>().join(" "))
                    .collect(),
            }
        })
        .collect();
    mine.extend(
        actions
            .iter()
            .filter(|a| a.provenance == Provenance::User)
            .map(|a| action_item(a, names)),
    );
    push(s.my_notes.into(), mine);
    Ok(sections)
}

fn action_item(a: &ghi_store::store::ActionItem, names: &HashMap<String, String>) -> Item {
    Item::Action {
        text: a.text.clone(),
        owner: a.owner_speaker_gid.as_deref().map(|g| name_of(names, g)),
        due: a
            .due_text
            .clone()
            .filter(|d| !d.trim().is_empty())
            .or_else(|| a.due.map(date_of)),
        done: a.done,
    }
}

/// Escapes user text for Markdown so it stays text: line-start block syntax
/// (`#`, `>`, `-`, `+`, `=`, `~`, `_`, `1.`), inline `\ * ` [ ] < &`. Line
/// breaks are kept; leading indentation is dropped (it would make code).
fn md_escape(s: &str) -> String {
    s.lines()
        .map(|line| md_escape_line(line.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn md_escape_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 4);
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    let numbered = digits > 0 && matches!(line[digits..].chars().next(), Some('.' | ')'));
    for (i, c) in line.chars().enumerate() {
        let block = i == 0 && matches!(c, '#' | '>' | '-' | '+' | '=' | '~' | '_');
        let list_marker = numbered && i == digits;
        if block || list_marker || matches!(c, '\\' | '*' | '`' | '[' | ']' | '<' | '&') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::sync::Arc;

    use ghi_store::keys::{MemoryKeyStore, Protection};
    use ghi_store::store::{
        NewActionItem, NewMeeting, NewNoteBlock, NewSegment, NewSpeaker, Provenance,
    };

    use super::*;

    const AN: &str = "Nguyễn Văn An";

    fn opts() -> ExportOptions {
        ExportOptions {
            include_notes: true,
            include_transcript: true,
            ui_lang: Lang::En,
        }
    }

    /// 2026-10-02 09:30 UTC.
    const START: i64 = 1_790_933_400_000;

    fn seg(speaker: &str, t0: i64, t1: i64, text: &str) -> NewSegment {
        NewSegment {
            speaker_gid: Some(speaker.to_string()),
            t0_ms: t0,
            t1_ms: t1,
            text: text.into(),
            ..Default::default()
        }
    }

    fn fixture() -> (tempfile::TempDir, Store, String) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        let m = store
            .create_meeting(NewMeeting {
                title: "Weekly sync".into(),
                started_at: START,
                source: "live".into(),
                mode: "room".into(),
                template: Some("standup".into()),
                ..Default::default()
            })
            .unwrap();
        let g = m.gid;
        let an = store
            .add_speaker(
                &g,
                NewSpeaker {
                    label_idx: 0,
                    display_name: Some(AN.into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let sp2 = store
            .add_speaker(
                &g,
                NewSpeaker {
                    label_idx: 1,
                    color_slot: 1,
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .add_segments(
                &g,
                vec![
                    seg(&an, 1_000, 2_500, "Chào mọi người, hôm nay chốt lịch beta."),
                    seg(&an, 2_500, 4_000, "Ship by Friday."),
                    seg(&sp2, 5_000, 6_000, "R&D agrees --> go <b>now</b>"),
                    seg(&an, 7_000, 8_000, "# not a heading"),
                ],
            )
            .unwrap();
        store.finish_meeting(&g, 725_000).unwrap();
        let ai = |kind: &str, body: &str| NewNoteBlock {
            kind: kind.into(),
            provenance: Provenance::Ai,
            body: body.into(),
            anchors: vec![],
            pinned: false,
        };
        let mine = store
            .add_note_block(
                &g,
                NewNoteBlock {
                    kind: "note".into(),
                    provenance: Provenance::User,
                    body: "ship date?".into(),
                    anchors: vec![],
                    pinned: false,
                },
            )
            .unwrap();
        for (k, b) in [
            (&mine.gid, "Confirmed for Friday"),
            (&mine.gid, ""),
            (&"gone".to_string(), "Orphan point"),
        ] {
            store
                .add_note_block(&g, ai(&format!("{ENHANCED_PREFIX}{k}"), b))
                .unwrap();
        }
        for b in [
            ai("tldr", "Chốt lịch beta"),
            ai("decision", "Ship by Friday"),
            ai("proposal", "Maybe a dark theme"),
            ai("answer", "Q: Who owns QA?\nA: Nam, from Monday."),
            ai("question", "Who owns QA?"),
            ai("section:done", "Wrote the parser"),
            NewNoteBlock {
                kind: "note".into(),
                provenance: Provenance::User,
                body: "- remember the demo".into(),
                anchors: vec![],
                pinned: false,
            },
        ] {
            store.add_note_block(&g, b).unwrap();
        }
        let done = store
            .add_action_item(
                &g,
                NewActionItem {
                    text: "Send the recap".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        store.set_action_done(&done.gid, true).unwrap();
        store
            .add_action_item(
                &g,
                NewActionItem {
                    text: "Deploy beta".into(),
                    owner_speaker_gid: Some(an),
                    due_text: Some("thứ Sáu".into()),
                    provenance: Provenance::Ai,
                    ..Default::default()
                },
            )
            .unwrap();
        (tmp, store, g)
    }

    fn render_str(store: &Store, g: &str, f: Format, o: &ExportOptions) -> String {
        String::from_utf8(render(store, g, f, o).unwrap()).unwrap()
    }

    #[test]
    fn markdown_structure_and_escaping() {
        let (_t, store, g) = fixture();
        let md = render_str(&store, &g, Format::Markdown, &opts());
        assert!(
            md.starts_with("# Weekly sync\n\n**Date:** 2026-10-02 09:30 UTC"),
            "{md}"
        );
        assert!(md.contains("**Duration:** 12:05"));
        assert!(md.contains(&format!("**Participants:** {AN}, Speaker 2")));
        assert!(md.contains("## Summary\n\n- Chốt lịch beta\n"));
        assert!(md.contains("## Done\n\n- Wrote the parser\n"));
        assert!(md.contains("## Decisions\n\n- Ship by Friday\n"));
        // A suggestion that nobody accepted is not under Decisions.
        assert!(md.contains("## Proposed\n\n- Maybe a dark theme\n"), "{md}");
        assert!(!md.contains("## Decisions\n\n- Ship by Friday\n- Maybe"));
        assert!(md.contains("- [x] Send the recap"));
        assert!(md.contains(&format!("- [ ] Deploy beta — {AN} (due thứ Sáu)")));
        assert!(md.contains("## Open questions"));
        assert!(
            md.contains("## Saved answers\n\n- Q: Who owns QA?\n  - A: Nam, from Monday."),
            "{md}"
        );
        assert!(md.contains("## My notes\n\n- ship date?"));
        assert!(md.contains("- \\- remember the demo"));
        // Consecutive lines of one speaker are one paragraph.
        assert!(md.contains(&format!(
            "**{AN}** [00:01]\n\nChào mọi người, hôm nay chốt lịch beta. Ship by Friday.\n"
        )));
        assert!(md.contains("**Speaker 2** [00:05]\n\nR\\&D agrees --> go \\<b>now\\</b>"));
        assert!(md.contains("\\# not a heading"));
    }

    #[test]
    fn markdown_vi_headings_and_toggles() {
        let (_t, store, g) = fixture();
        let o = ExportOptions {
            ui_lang: Lang::Vi,
            include_transcript: false,
            ..opts()
        };
        let md = render_str(&store, &g, Format::Markdown, &o);
        assert!(md.contains("## Tóm tắt"));
        assert!(md.contains("## Việc cần làm"));
        assert!(md.contains("## Đề xuất\n\n- Maybe a dark theme"));
        assert!(md.contains("## Câu trả lời đã lưu"));
        assert!(md.contains("(hạn thứ Sáu)"));
        assert!(!md.contains("Bản ghi"));
        let o = ExportOptions {
            include_notes: false,
            ..opts()
        };
        let md = render_str(&store, &g, Format::Markdown, &o);
        assert!(!md.contains("## Summary"));
        assert!(md.contains("## Transcript"));
    }

    #[test]
    fn plain_text_has_no_markdown() {
        let (_t, store, g) = fixture();
        let t = render_str(&store, &g, Format::Text, &opts());
        assert!(t.starts_with("Weekly sync\n\nDate: 2026-10-02 09:30 UTC\n"));
        assert!(t.contains("\nSummary\n\n• Chốt lịch beta\n"));
        assert!(t.contains("\nProposed\n\n• Maybe a dark theme\n"), "{t}");
        assert!(t.contains("\nSaved answers\n\n• Q: Who owns QA?\n"), "{t}");
        assert!(t.contains("A: Nam, from Monday."), "{t}");
        assert!(t.contains("☑ Send the recap"));
        assert!(t.contains(&format!("☐ Deploy beta — {AN} (due thứ Sáu)")));
        assert!(t.contains("\nSpeaker 2 [00:05]\nR&D agrees --> go <b>now</b>\n"));
        assert!(!t.contains("**") && !t.contains("##") && !t.contains("\\"));
    }

    #[test]
    fn srt_timestamps_and_arrows() {
        assert_eq!(cue_time(3_723_456, ','), "01:02:03,456");
        assert_eq!(cue_time(3_723_456, '.'), "01:02:03.456");
        let (_t, store, g) = fixture();
        let srt = render_str(&store, &g, Format::Srt, &opts());
        assert!(srt.starts_with(&format!(
            "1\n00:00:01,000 --> 00:00:02,500\n{AN}: Chào mọi người, hôm nay chốt lịch beta.\n\n2\n"
        )));
        assert!(srt.contains("Speaker 2: R&D agrees -> go <b>now</b>\n"));
        assert_eq!(srt.matches("-->").count(), 4, "only the cue arrows");
        assert!(!srt.contains("## Summary"));
    }

    #[test]
    fn vtt_escapes_cue_text() {
        let (_t, store, g) = fixture();
        let vtt = render_str(&store, &g, Format::Vtt, &opts());
        assert!(vtt.starts_with("WEBVTT\n\n00:00:01.000 --> 00:00:02.500\n"));
        assert!(vtt.contains("Speaker 2: R&amp;D agrees --&gt; go &lt;b&gt;now&lt;/b&gt;\n"));
        assert_eq!(vtt.matches("-->").count(), 4);
        assert!(!vtt.contains("<b>"));
    }

    #[test]
    fn subtitles_of_an_empty_transcript_are_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        let g = store
            .create_meeting(NewMeeting {
                title: String::new(),
                source: "live".into(),
                mode: "room".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        assert_eq!(render_str(&store, &g, Format::Vtt, &opts()), "WEBVTT\n\n");
        assert_eq!(render_str(&store, &g, Format::Srt, &opts()), "");
        let md = render_str(&store, &g, Format::Markdown, &opts());
        assert!(!md.contains("## "));
    }

    #[test]
    fn file_stem_is_safe() {
        let (_t, store, g) = fixture();
        let mut m = store.get_meeting(&g).unwrap();
        assert_eq!(file_stem(&m), "2026-10-02 Weekly sync");
        m.title = "../a/b:c".into();
        assert_eq!(file_stem(&m), "2026-10-02 a b c");
        m.title = "  ...  ".into();
        assert_eq!(file_stem(&m), "2026-10-02 Meeting");
        m.title = String::new();
        assert_eq!(file_stem(&m), "2026-10-02 Meeting");
        m.title = "x".repeat(200);
        assert_eq!(file_stem(&m).chars().count(), 80);
        m.title = "Cuộc họp \"quý\" ?".into();
        assert_eq!(file_stem(&m), "2026-10-02 Cuộc họp quý");
        // NFC: decomposed input comes out composed.
        m.title = "Ngu\u{0303}y\u{0302}\u{0303}".into();
        assert_eq!(file_stem(&m), ghi_text::nfc(&file_stem(&m)));
    }

    #[test]
    fn enhanced_notes_nest_under_my_notes() {
        let (_t, store, g) = fixture();
        let md = render_str(&store, &g, Format::Markdown, &opts());
        assert!(
            md.contains("- ship date?\n  - Confirmed for Friday\n"),
            "{md}"
        );
        assert_eq!(md.matches("Confirmed for Friday").count(), 1);
        let t = render_str(&store, &g, Format::Text, &opts());
        assert!(
            t.contains("• ship date?\n    • Confirmed for Friday\n"),
            "{t}"
        );
        for out in [&md, &t] {
            assert!(!out.contains("Orphan point") && !out.contains("enhanced:"));
        }
        let bytes = render(&store, &g, Format::Docx, &opts()).unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        zip.by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        assert!(xml.contains("• Confirmed for Friday") && !xml.contains("Orphan point"));
        assert!(xml.contains("w:left=\"720\""), "{xml}");
    }

    #[test]
    fn extensions() {
        assert_eq!(extension(Format::Docx), "docx");
        assert_eq!(extension(Format::Markdown), "md");
    }

    #[test]
    fn obsidian_front_matter_and_no_overwrite() {
        let (_t, store, g) = fixture();
        let dir = tempfile::tempdir().unwrap();
        let n1 = write_obsidian(&store, &g, dir.path(), &opts()).unwrap();
        let n2 = write_obsidian(&store, &g, dir.path(), &opts()).unwrap();
        let n3 = write_obsidian(&store, &g, dir.path(), &opts()).unwrap();
        assert_eq!(n1, "2026-10-02 Weekly sync.md");
        assert_eq!(n2, "2026-10-02 Weekly sync (2).md");
        assert_eq!(n3, "2026-10-02 Weekly sync (3).md");
        let body = std::fs::read_to_string(dir.path().join(&n1)).unwrap();
        assert!(body.starts_with(&format!(
            "---\ntitle: \"Weekly sync\"\ndate: 2026-10-02T09:30\nduration: \"12:05\"\nparticipants: [\"{AN}\", \"Speaker 2\"]\ntags: [ghira]\n---\n\n# Weekly sync"
        )), "{body}");
        assert!(
            body.contains("## Proposed\n\n- Maybe a dark theme"),
            "{body}"
        );
    }

    #[test]
    fn obsidian_tag_names_are_made_safe() {
        assert_eq!(obsidian_tag("Q4 plan").as_deref(), Some("Q4-plan"));
        assert_eq!(
            obsidian_tag("  #urgent, now ").as_deref(),
            Some("urgent-now")
        );
        assert_eq!(obsidian_tag("Họp").as_deref(), Some("Họp"));
        assert_eq!(
            obsidian_tag("clients/Acme").as_deref(),
            Some("clients/Acme")
        );
        assert_eq!(obsidian_tag("a  --  b").as_deref(), Some("a-b"));
        assert_eq!(
            obsidian_tag("\"quoted\" [x]: y").as_deref(),
            Some("quoted-x-y")
        );
        // Nothing left, or only digits: Obsidian would not take it.
        assert_eq!(obsidian_tag("#, ,"), None);
        assert_eq!(obsidian_tag("2026"), None);
    }

    #[test]
    fn obsidian_front_matter_has_the_meeting_tags() {
        let (_t, store, g) = fixture();
        for name in ["Q4 plan", "Họp", "q4  PLAN", "2026"] {
            let t = store.create_tag(name).unwrap();
            store
                .tag_meetings(std::slice::from_ref(&g), &t.gid)
                .unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let n = write_obsidian(&store, &g, dir.path(), &opts()).unwrap();
        let body = std::fs::read_to_string(dir.path().join(n)).unwrap();
        let tags_line = body.lines().find(|l| l.starts_with("tags:")).unwrap();
        // "q4  PLAN" is the same tag as "Q4 plan" (names compare ignoring case): one entry, "2026" is dropped.
        assert_eq!(tags_line, "tags: [ghira, Họp, Q4-plan]", "{body}");
        let originals = body.lines().find(|l| l.starts_with("ghira_tags:")).unwrap();
        assert_eq!(
            originals, "ghira_tags: [\"2026\", \"Họp\", \"Q4 plan\"]",
            "{body}"
        );
        // Other exports do not carry tags.
        let md = render(&store, &g, Format::Markdown, &opts()).unwrap();
        assert!(!String::from_utf8(md).unwrap().contains("ghira_tags"));
    }

    #[test]
    fn docx_round_trips_vietnamese() {
        let (_t, store, g) = fixture();
        let bytes = render(&store, &g, Format::Docx, &opts()).unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        zip.by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        assert!(xml.contains(AN), "{xml}");
        assert!(xml.contains("Chào mọi người, hôm nay chốt lịch beta."));
        assert!(xml.contains("R&amp;D agrees --&gt; go &lt;b&gt;now&lt;/b&gt;"));
        assert!(xml.contains("☑ Send the recap"));
        assert!(xml.contains("Proposed") && xml.contains("Maybe a dark theme"));
        assert!(xml.contains("Saved answers") && xml.contains("Nam, from Monday."));
        assert!(xml.contains("w:val=\"Title\"") && xml.contains("w:val=\"Heading1\""));
        let mut styles = String::new();
        zip.by_name("word/styles.xml")
            .unwrap()
            .read_to_string(&mut styles)
            .unwrap();
        assert!(styles.contains("Heading1") && styles.contains("Title"));
    }

    #[test]
    fn civil_dates() {
        assert_eq!(datetime_of(0), "1970-01-01 00:00");
        assert_eq!(datetime_of(START), "2026-10-02 09:30");
        assert_eq!(date_of(951_782_400_000), "2000-02-29");
        assert_eq!(date_of(-86_400_000), "1969-12-31");
    }
}
