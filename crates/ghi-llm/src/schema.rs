// SPDX-License-Identifier: Apache-2.0
//! JSON schemas of what the model must return, generated from templates.
//!
//! The same schema drives constrained decoding in the local worker (llama.cpp
//! turns it into a grammar) and the JSON modes of cloud providers. Schemas are
//! flat, every property is required and extra properties are refused, which is
//! the subset all three cloud providers accept. `Dialect::Cloud` also leaves out
//! array-size and number bounds (Anthropic rejects them); the validator
//! enforces those limits either way.
//!
//! Citations are `cite: [segment id, ...]`. For the local model the ids are an
//! enum of the segments actually in the prompt and speaker fields an enum of the
//! speaker aliases (`SPK1`, `SPK2`, ...), so the grammar can't produce an id or an
//! owner that doesn't exist; the validator checks the same for cloud output.

use serde_json::{Value, json};

use crate::template::Template;

/// Top-level keys every notes output has; template sections can't reuse them.
pub const CORE_KEYS: &[&str] = &[
    "tldr",
    "decisions",
    "action_items",
    "open_questions",
    "key_quotes",
    "topics",
];

/// At most this many TL;DR bullets (doc 02 §E).
pub const MAX_TLDR: usize = 5;
/// At most this many citations per item.
pub const MAX_CITES: usize = 8;
pub const MAX_QUOTES: usize = 5;
pub const MAX_TOPICS: usize = 12;
/// Facts one map step may return.
pub const MAX_FACTS: usize = 40;
/// Supporting points per enhanced note line.
pub const MAX_POINTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// The local worker: bounds are enforced by the grammar.
    Local,
    /// Cloud JSON modes: no array or number bounds.
    Cloud,
}

/// What a schema may refer to: the prompt's segment ids and speaker aliases.
#[derive(Debug, Clone, Copy)]
pub struct Shape<'a> {
    pub dialect: Dialect,
    pub ids: &'a [u64],
    pub speakers: &'a [String],
}

fn object(props: &[(&str, Value)]) -> Value {
    let mut properties = serde_json::Map::new();
    for (k, v) in props {
        properties.insert((*k).to_string(), v.clone());
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": props.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
        "additionalProperties": false,
    })
}

fn array(items: Value, min: Option<usize>, max: Option<usize>, d: Dialect) -> Value {
    let mut a = json!({"type": "array", "items": items});
    if d == Dialect::Local {
        if let Some(min) = min {
            a["minItems"] = json!(min);
        }
        if let Some(max) = max {
            a["maxItems"] = json!(max);
        }
    }
    a
}

fn string() -> Value {
    json!({"type": "string"})
}

fn nullable_string() -> Value {
    json!({"type": ["string", "null"]})
}

/// A speaker alias from `speakers`, or null.
fn speaker(speakers: &[String]) -> Value {
    if speakers.is_empty() {
        return json!({"type": "null"});
    }
    let mut options: Vec<Value> = speakers.iter().map(|s| json!(s)).collect();
    options.push(Value::Null);
    json!({"type": ["string", "null"], "enum": options})
}

/// Segment ids cited by an item: at least one (if `required`), at most
/// [`MAX_CITES`]; locally only ids that are in the prompt.
fn cites(s: &Shape, required: bool) -> Value {
    let id = match s.dialect {
        Dialect::Local if !s.ids.is_empty() => json!({"type": "integer", "enum": s.ids}),
        _ => json!({"type": "integer"}),
    };
    array(id, required.then_some(1), Some(MAX_CITES), s.dialect)
}

fn item(s: &Shape) -> Value {
    object(&[("text", string()), ("cite", cites(s, true))])
}

/// The notes output for `template`.
pub fn notes(template: &Template, s: &Shape) -> Value {
    let d = s.dialect;
    let action = object(&[
        ("text", string()),
        ("owner", speaker(s.speakers)),
        ("due", nullable_string()),
        ("cite", cites(s, true)),
    ]);
    let quote = object(&[
        ("text", string()),
        ("speaker", speaker(s.speakers)),
        ("cite", cites(s, true)),
    ]);
    let topic = object(&[("title", string()), ("cite", cites(s, true))]);
    let mut props = vec![
        ("tldr", array(item(s), None, Some(MAX_TLDR), d)),
        ("decisions", array(item(s), None, None, d)),
        ("action_items", array(action, None, None, d)),
        ("open_questions", array(item(s), None, None, d)),
        ("key_quotes", array(quote, None, Some(MAX_QUOTES), d)),
        ("topics", array(topic, None, Some(MAX_TOPICS), d)),
    ];
    for sec in &template.sections {
        props.push((sec.id.as_str(), array(item(s), None, None, d)));
    }
    object(&props)
}

/// Kinds of facts the map step extracts from one chunk.
pub const FACT_KINDS: &[&str] = &["decision", "action", "question", "quote", "point"];

/// The map step: facts from one chunk of the transcript.
pub fn facts(s: &Shape) -> Value {
    let fact = object(&[
        ("kind", json!({"type": "string", "enum": FACT_KINDS})),
        ("text", string()),
        ("speaker", speaker(s.speakers)),
        ("owner", speaker(s.speakers)),
        ("due", nullable_string()),
        ("cite", cites(s, true)),
    ]);
    object(&[("facts", array(fact, None, Some(MAX_FACTS), s.dialect))])
}

/// Enhance: per user note line (numbered from 1), supporting points or not found.
pub fn enhance(s: &Shape, lines: usize) -> Value {
    let line_no = match s.dialect {
        Dialect::Local => json!({"type": "integer", "enum": (1..=lines).collect::<Vec<_>>()}),
        Dialect::Cloud => json!({"type": "integer"}),
    };
    let line = object(&[
        ("line", line_no),
        ("found", json!({"type": "boolean"})),
        ("points", array(item(s), None, Some(MAX_POINTS), s.dialect)),
    ]);
    object(&[("lines", array(line, None, Some(lines), s.dialect))])
}

/// Ask this meeting: an answer with citations, or not discussed.
pub fn ask(s: &Shape) -> Value {
    object(&[
        ("discussed", json!({"type": "boolean"})),
        ("answer", string()),
        ("cite", cites(s, false)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template;

    fn walk(v: &Value, f: &mut impl FnMut(&serde_json::Map<String, Value>)) {
        match v {
            Value::Object(m) => {
                f(m);
                m.values().for_each(|c| walk(c, f));
            }
            Value::Array(a) => a.iter().for_each(|c| walk(c, f)),
            _ => {}
        }
    }

    fn speakers(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_object_requires_all_properties_and_no_extras() {
        let t = template::builtin("standup").unwrap();
        let sp = speakers(&["S1", "S2"]);
        for dialect in [Dialect::Local, Dialect::Cloud] {
            let sh = Shape {
                dialect,
                ids: &[0, 4, 7],
                speakers: &sp,
            };
            for s in [notes(&t, &sh), facts(&sh), enhance(&sh, 3), ask(&sh)] {
                walk(&s, &mut |m| {
                    if m.get("type") == Some(&json!("object")) {
                        assert_eq!(m["additionalProperties"], json!(false));
                        let props: Vec<_> = m["properties"].as_object().unwrap().keys().collect();
                        assert_eq!(m["required"].as_array().unwrap().len(), props.len());
                    }
                });
            }
        }
    }

    #[test]
    fn cloud_dialect_has_no_bounds_or_id_enums() {
        let t = template::builtin("general").unwrap();
        let sp = speakers(&["S1"]);
        let mut sh = Shape {
            dialect: Dialect::Cloud,
            ids: &[1, 2],
            speakers: &sp,
        };
        let s = notes(&t, &sh).to_string();
        for bound in ["minItems", "maxItems", "minimum", "[1,2]"] {
            assert!(!s.contains(bound), "{bound}");
        }
        sh.dialect = Dialect::Local;
        let local = notes(&t, &sh).to_string();
        assert!(local.contains("\"maxItems\":5"));
        assert!(local.contains("\"enum\":[1,2]"));
    }

    #[test]
    fn template_sections_become_keys_and_speakers_an_enum() {
        let t = template::builtin("standup").unwrap();
        let sp = speakers(&["S1", "S2"]);
        let sh = Shape {
            dialect: Dialect::Local,
            ids: &[],
            speakers: &sp,
        };
        let s = notes(&t, &sh);
        for k in CORE_KEYS.iter().chain(["done", "next", "blockers"].iter()) {
            assert!(s["properties"].get(*k).is_some(), "{k}");
        }
        let owner = &s["properties"]["action_items"]["items"]["properties"]["owner"];
        assert_eq!(owner["enum"], json!(["S1", "S2", null]));
        let none = notes(
            &t,
            &Shape {
                speakers: &[],
                ..sh
            },
        );
        assert_eq!(
            none["properties"]["action_items"]["items"]["properties"]["owner"],
            json!({"type": "null"})
        );
    }
}
