// SPDX-License-Identifier: Apache-2.0
//! Whitelist scrubber for the one free-text field of a report (the panic
//! message). It cannot know what is private, so it keeps only what looks like
//! a short developer message and replaces everything that could be content.

/// Longest scrubbed message.
const MAX_CHARS: usize = 300;
/// Quoted spans longer than this are replaced.
const MAX_QUOTED: usize = 40;
/// Hex runs at least this long (keys, hashes) are replaced.
const MIN_HEX: usize = 32;

/// Scrubs a possibly multi-line message: first line only.
pub fn scrub(msg: &str) -> String {
    scrub_line(msg.lines().next().unwrap_or(""))
}

/// Scrubs one line (anything after a newline is dropped).
pub fn scrub_line(msg: &str) -> String {
    let line = msg.split(['\n', '\r']).next().unwrap_or("");
    let line = replace_home(line);
    let line = replace_quoted(&line);
    let line = replace_non_ascii_words(&line);
    let line = replace_hex(&line);
    let line: String = line.chars().filter(|c| !c.is_control()).collect();
    truncate(line)
}

fn replace_home(s: &str) -> String {
    match std::env::var("HOME") {
        Ok(h) if h.len() > 1 => s.replace(&h, "~"),
        _ => s.to_owned(),
    }
}

/// "…", '…' and `…` longer than [`MAX_QUOTED`] become `<text>`. An unclosed
/// quote swallows the rest of the line when that is long.
fn replace_quoted(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let q = chars[i];
        if !matches!(q, '"' | '\'' | '`') {
            out.push(q);
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let mut closed = false;
        while j < chars.len() {
            if chars[j] == '\\' {
                j += 2;
                continue;
            }
            if chars[j] == q {
                closed = true;
                break;
            }
            j += 1;
        }
        let end = j.min(chars.len());
        let inner = end.saturating_sub(i + 1);
        if inner > MAX_QUOTED {
            out.push_str("<text>");
            i = if closed { end + 1 } else { chars.len() };
        } else {
            // Short span (an identifier, a short enum value): keep, move on.
            out.push(q);
            i += 1;
        }
    }
    out
}

/// Panic messages written by us are ASCII English; a word with any other
/// character is transcript or user text.
fn replace_non_ascii_words(s: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if !word.is_empty() {
            if word.is_ascii() {
                out.push_str(word);
            } else {
                out.push_str("<text>");
            }
            word.clear();
        }
    };
    for c in s.chars() {
        if c.is_whitespace() {
            flush(&mut word, &mut out);
            out.push(c);
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn replace_hex(s: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    for c in s.chars().chain(std::iter::once('\0')) {
        if c.is_ascii_hexdigit() {
            run.push(c);
            continue;
        }
        if run.len() >= MIN_HEX {
            out.push_str("<hex>");
        } else {
            out.push_str(&run);
        }
        run.clear();
        if c != '\0' {
            out.push(c);
        }
    }
    out
}

fn truncate(s: String) -> String {
    if s.chars().count() <= MAX_CHARS {
        return s;
    }
    s.chars().take(MAX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    #[test]
    fn hex_key_is_replaced_even_unquoted() {
        let out = scrub(&format!("bad key {KEY} for store"));
        assert!(!out.contains("9f86d081"), "{out}");
        assert_eq!(out, "bad key <hex> for store");
    }

    #[test]
    fn short_hex_is_kept() {
        assert_eq!(scrub("id deadbeef failed"), "id deadbeef failed");
    }

    #[test]
    fn vietnamese_text_never_survives() {
        let quoted = "called `Result::unwrap()` on an `Err` value: \"Chúng ta sẽ họp lại vào thứ Sáu để chốt ngân sách quý bốn\"";
        let out = scrub(quoted);
        assert!(!out.contains("ngân sách"), "{out}");
        assert!(!out.contains("Chúng"), "{out}");
        let bare = scrub("segment Chúng ta sẽ họp lại");
        assert!(!bare.contains("họp"), "{bare}");
        assert!(!bare.contains("Chúng"), "{bare}");
    }

    #[test]
    fn short_quotes_are_kept_long_are_not() {
        assert_eq!(scrub("missing field `speaker`"), "missing field `speaker`");
        let long = format!("x '{}' y", "a".repeat(41));
        assert_eq!(scrub(&long), "x <text> y");
        let unclosed = format!("x \"{}", "b".repeat(60));
        assert_eq!(scrub(&unclosed), "x <text>");
    }

    #[test]
    fn first_line_only_controls_stripped_and_capped() {
        assert_eq!(scrub("one\ntwo"), "one");
        assert_eq!(scrub("a\u{7}b\tc"), "abc");
        assert_eq!(scrub(&"z".repeat(1000)).chars().count(), 300);
    }

    #[test]
    fn home_becomes_tilde() {
        let home = std::env::var("HOME").unwrap_or_default();
        if home.len() > 1 {
            let out = scrub(&format!("cannot open {home}/Library/x"));
            assert_eq!(out, "cannot open ~/Library/x");
        }
    }
}
