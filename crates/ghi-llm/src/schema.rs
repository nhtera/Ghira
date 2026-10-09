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

/// A speaker alias from `speakers`, or null. Cloud schemas leave out the
/// enum (Anthropic rejects an enum next to a `["string", "null"]` type); the
/// validator keeps only known aliases either way.
fn speaker(s: &Shape) -> Value {
    let speakers = s.speakers;
    if speakers.is_empty() {
        return json!({"type": "null"});
    }
    if s.dialect == Dialect::Cloud {
        return nullable_string();
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

/// How sure a decision is: someone agreed or confirmed it, or it was only suggested.
pub const DECISION_STATUSES: &[&str] = &["decided", "proposed"];

/// A decision: like an item, with its status (required, like every property).
fn decision(s: &Shape) -> Value {
    object(&[
        ("text", string()),
        (
            "status",
            json!({"type": "string", "enum": DECISION_STATUSES}),
        ),
        ("cite", cites(s, true)),
    ])
}

/// The notes output for `template`.
pub fn notes(template: &Template, s: &Shape) -> Value {
    let d = s.dialect;
    let action = object(&[
        ("text", string()),
        ("owner", speaker(s)),
        ("due", nullable_string()),
        ("cite", cites(s, true)),
    ]);
    let quote = object(&[
        ("text", string()),
        ("speaker", speaker(s)),
        ("cite", cites(s, true)),
    ]);
    let topic = object(&[("title", string()), ("cite", cites(s, true))]);
    let mut props = vec![
        ("tldr", array(item(s), None, Some(MAX_TLDR), d)),
        ("decisions", array(decision(s), None, None, d)),
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

/// Item caps of a compact notes schema (a phone writes these instead of the
/// full notes: about half the output, so half the time and heat). Quotes and
/// topics are left empty; the template's own sections keep a few items.
pub const COMPACT_CAPS: &[(&str, usize)] = &[
    ("tldr", 4),
    ("decisions", 6),
    ("action_items", 8),
    ("open_questions", 4),
    ("key_quotes", 0),
    ("topics", 0),
];
/// Items per template section in a compact schema.
pub const COMPACT_SECTION_CAP: usize = 4;
/// Facts per part and citations per fact in a compact map step (a phone part
/// once wrote 40 facts, ~2,000 tokens, ~5 min).
pub const COMPACT_FACTS: usize = 8;
pub const COMPACT_CITES: usize = 3;
/// The answer cap of a compact map step, in tokens.
pub const COMPACT_FACTS_TOKENS: u32 = 700;

/// The compact form of a local [`facts`] schema: at most [`COMPACT_FACTS`]
/// facts of at most [`COMPACT_CITES`] citations.
pub fn compact_facts(mut schema: Value, d: Dialect) -> Value {
    if d != Dialect::Local {
        return schema;
    }
    let facts = &mut schema["properties"]["facts"];
    facts["maxItems"] = json!(COMPACT_FACTS);
    facts["items"]["properties"]["cite"]["maxItems"] = json!(COMPACT_CITES);
    schema
}

/// The compact form of a local [`notes`] schema (`maxItems` lowered; cloud
/// schemas carry no bounds and are returned as they are).
pub fn compact(mut schema: Value, d: Dialect) -> Value {
    if d != Dialect::Local {
        return schema;
    }
    if let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        for (key, prop) in props.iter_mut() {
            let cap = COMPACT_CAPS
                .iter()
                .find(|(k, _)| k == key)
                .map_or(COMPACT_SECTION_CAP, |(_, cap)| *cap);
            let lower = prop
                .get("maxItems")
                .and_then(Value::as_u64)
                .map_or(cap, |m| (m as usize).min(cap));
            prop["maxItems"] = json!(lower);
        }
    }
    schema
}

/// Kinds of facts the map step extracts from one chunk.
pub const FACT_KINDS: &[&str] = &[
    "decision", "proposal", "action", "question", "quote", "point",
];

/// The map step: facts from one chunk of the transcript.
pub fn facts(s: &Shape) -> Value {
    let fact = object(&[
        ("kind", json!({"type": "string", "enum": FACT_KINDS})),
        ("text", string()),
        ("speaker", speaker(s)),
        ("owner", speaker(s)),
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
/// A note template drafted from a description (the editor's shape; ids are
/// made when it is saved). Local only: the grammar bounds the sections.
pub fn template_draft() -> Value {
    let section = object(&[("title", string()), ("instruction", string())]);
    object(&[
        ("name", string()),
        ("guidance", string()),
        (
            "sections",
            array(
                section,
                Some(1),
                Some(crate::template::MAX_SECTIONS),
                Dialect::Local,
            ),
        ),
    ])
}

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
    fn compact_notes_leave_quotes_and_topics_empty_and_cap_the_rest() {
        let t = template::builtin("general").unwrap();
        let sp = speakers(&["S1"]);
        let sh = Shape {
            dialect: Dialect::Local,
            ids: &[0, 1],
            speakers: &sp,
        };
        let c = compact(notes(&t, &sh), Dialect::Local);
        let max = |k: &str| c["properties"][k]["maxItems"].as_u64();
        assert_eq!(max("key_quotes"), Some(0));
        assert_eq!(max("topics"), Some(0));
        assert_eq!(max("tldr"), Some(4));
        assert_eq!(max("action_items"), Some(8));
        // Still every key, still required: the parser is unchanged.
        assert_eq!(c["required"], notes(&t, &sh)["required"]);
        // Compact map steps: few facts, few citations each.
        let f = compact_facts(facts(&sh), Dialect::Local);
        assert_eq!(f["properties"]["facts"]["maxItems"], COMPACT_FACTS);
        assert_eq!(
            f["properties"]["facts"]["items"]["properties"]["cite"]["maxItems"],
            COMPACT_CITES
        );
        // Cloud schemas carry no bounds.
        let cloud = Shape {
            dialect: Dialect::Cloud,
            ..sh
        };
        assert_eq!(
            compact(notes(&t, &cloud), Dialect::Cloud),
            notes(&t, &cloud)
        );
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
        // No enum next to a ["string","null"] type (the speakers): Anthropic
        // rejects it; aliases are checked when the reply is parsed. The only
        // enum is the decision status, on a plain string.
        for bound in ["minItems", "maxItems", "minimum", "[1,2]"] {
            assert!(!s.contains(bound), "{bound}");
        }
        assert_eq!(s.matches("\"enum\"").count(), 1);
        assert!(s.contains(r#""enum":["decided","proposed"]"#));
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
