// SPDX-License-Identifier: Apache-2.0
//! Guard for the crash reports: a `panic!`, `unreachable!` or `expect(..)`
//! message in the crates that handle meeting content must not interpolate a
//! value named like content (text, segment, body, title, key, name). The
//! scrubber is the second line of defense; this is the first.

use std::path::{Path, PathBuf};

const CRATES: [&str; 2] = ["ghi-store", "ghi-core"];
const BANNED: [&str; 7] = [
    "text",
    "segment",
    "body",
    "title",
    "key",
    "name",
    "transcript",
];
const MACROS: [&str; 3] = ["panic!(", "unreachable!(", ".expect("];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The text between the parens that open at `open` (index of `(`), plus
/// whether string literals are skipped when looking at identifiers.
fn call_args(src: &str, open: usize) -> &str {
    let b = src.as_bytes();
    let (mut depth, mut i, mut in_str) = (0i32, open, false);
    while i < b.len() {
        match b[i] {
            b'\\' if in_str => i += 1,
            b'"' => in_str = !in_str,
            b'(' if !in_str => depth += 1,
            b')' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return &src[open + 1..i];
                }
            }
            _ => {}
        }
        i += 1;
    }
    &src[open + 1..]
}

fn banned(ident: &str) -> bool {
    ident
        .split('_')
        .any(|p| BANNED.contains(&p.to_ascii_lowercase().as_str()))
}

/// Identifiers interpolated by the call: `{ident}` inside literals and the
/// identifiers of arguments outside literals.
fn interpolated(args: &str) -> Vec<String> {
    let mut idents = Vec::new();
    let mut outside = String::new();
    let mut lit = String::new();
    let mut in_str = false;
    let mut chars = args.chars();
    while let Some(c) = chars.next() {
        match (c, in_str) {
            ('\\', true) => {
                chars.next();
            }
            ('"', _) => {
                if in_str {
                    // `{name}` / `{name:?}` placeholders of this literal.
                    for part in lit.split('{').skip(1) {
                        let id: String = part
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '_')
                            .collect();
                        if !id.is_empty() {
                            idents.push(id);
                        }
                    }
                    lit.clear();
                }
                in_str = !in_str;
            }
            (c, true) => lit.push(c),
            (c, false) => outside.push(c),
        }
    }
    idents.extend(
        outside
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|w| !w.is_empty() && !w.chars().next().unwrap().is_numeric())
            .map(str::to_owned),
    );
    idents
}

fn violations(file: &Path) -> Vec<String> {
    let full = std::fs::read_to_string(file).unwrap();
    // Test modules sit at the end of a file and may name anything.
    let src = full.split("#[cfg(test)]").next().unwrap();
    let mut found = Vec::new();
    for m in MACROS {
        for (at, _) in src.match_indices(m) {
            let args = call_args(src, at + m.len() - 1);
            // A plain `.expect("literal")` has no interpolation.
            let ids = interpolated(args);
            if let Some(id) = ids.iter().find(|i| banned(i)) {
                let line = src[..at].matches('\n').count() + 1;
                found.push(format!(
                    "{}:{line}: {m} interpolates `{id}`",
                    file.display()
                ));
            }
        }
    }
    found
}

#[test]
fn panic_messages_do_not_interpolate_content() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for c in CRATES {
        rs_files(&root.join(c).join("src"), &mut files);
    }
    assert!(!files.is_empty());
    let bad: Vec<String> = files.iter().flat_map(|f| violations(f)).collect();
    assert!(
        bad.is_empty(),
        "content may leak into crash reports:\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_check_itself_catches_interpolation() {
    let args = interpolated(r#""bad segment {text}", seg.title"#);
    assert!(args.iter().any(|i| banned(i)));
    assert!(
        !interpolated(r#""segment missing""#)
            .iter()
            .any(|i| banned(i))
    );
}
