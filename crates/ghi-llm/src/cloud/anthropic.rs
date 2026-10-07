// SPDX-License-Identifier: Apache-2.0
//! Anthropic Messages API with structured outputs.
//!
//! The schema goes in `output_config.format` (structured outputs, GA; the old
//! `output_format` field and beta header are deprecated). If a model does not
//! support it, the fallback is forced tool use (`tools` + `tool_choice`), not
//! implemented: the engine re-validates every answer anyway.

use serde_json::{Value, json};

use super::{Prepared, REASONING_MAX_TOKENS, Traits, excerpt};
use crate::{Completion, LlmError, Request, Result, Role};

pub(super) fn prepare(base_url: &str, model: &str, req: &Request, t: Traits) -> Result<Prepared> {
    // System messages are a top-level field; the rest alternate user/assistant.
    let system: Vec<&str> = req
        .messages
        .iter()
        .filter(|m| m.role == Role::System)
        .map(|m| m.content.as_str())
        .collect();
    let messages: Vec<Value> = req
        .messages
        .iter()
        .filter(|m| m.role != Role::System)
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
    if messages.is_empty() {
        return Err(LlmError::Invalid(
            "a cloud request needs a user message".into(),
        ));
    }
    let cap = if t.reasons {
        req.max_tokens.max(REASONING_MAX_TOKENS)
    } else {
        req.max_tokens
    };
    let mut body = json!({
        "model": model,
        "max_tokens": cap,
        "messages": messages,
    });
    // Claude 4.7 and later refuse a non-default temperature.
    if t.temperature {
        body["temperature"] = json!(req.temperature);
    }
    if !system.is_empty() {
        body["system"] = json!(system.join("\n\n"));
    }
    if let Some(schema) = &req.schema {
        body["output_config"] = json!({"format": {"type": "json_schema", "schema": schema}});
    }
    Ok(Prepared {
        url: format!("{}/v1/messages", base_url.trim_end_matches('/')),
        body: serde_json::to_vec(&body).expect("a JSON value serializes"),
    })
}

pub(super) fn parse(status: u16, json: &Value) -> Result<Completion> {
    let bad = |what: &str| LlmError::Provider {
        status,
        message: format!("unexpected answer: {what}"),
    };
    let stop = json.get("stop_reason").and_then(Value::as_str);
    let blocks = json
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("no content"))?;
    let text: String = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    if stop == Some("refusal") {
        return Err(LlmError::Provider {
            status,
            message: format!("the model refused: {}", excerpt(&text)),
        });
    }
    if text.is_empty() {
        return Err(bad("no text"));
    }
    let count = |key: &str| {
        json.pointer(&format!("/usage/{key}"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    // Cached prompt tokens are still prompt tokens (billed at other rates).
    let tokens_in = count("input_tokens")
        + count("cache_creation_input_tokens")
        + count("cache_read_input_tokens");
    Ok(Completion {
        text,
        tokens_in: u32::try_from(tokens_in).unwrap_or(u32::MAX),
        tokens_out: u32::try_from(count("output_tokens")).unwrap_or(u32::MAX),
        truncated: stop == Some("max_tokens"),
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::request;
    use super::super::{CloudProvider, LlmError};
    use crate::Message;
    use serde_json::Value;

    fn provider() -> CloudProvider {
        CloudProvider::preset("anthropic", "claude-haiku-4-5").unwrap()
    }

    #[test]
    fn newer_claude_gets_no_temperature_and_room_to_think() {
        let p = CloudProvider::preset("anthropic", "claude-sonnet-5-5").unwrap();
        let v: Value = serde_json::from_slice(&p.prepare(&request()).unwrap().body).unwrap();
        assert!(v.get("temperature").is_none(), "{v}");
        assert_eq!(v["max_tokens"], 16_384);
        assert_eq!(v["output_config"]["format"]["type"], "json_schema");
    }

    #[test]
    fn request_shape() {
        let prepared = provider().prepare(&request()).unwrap();
        assert_eq!(prepared.url, "https://api.anthropic.com/v1/messages");
        let v: Value = serde_json::from_slice(&prepared.body).unwrap();
        assert_eq!(v["model"], "claude-haiku-4-5");
        assert_eq!(v["max_tokens"], 2048);
        assert!(v["temperature"].as_f64().is_some());
        assert_eq!(v["system"], "You write meeting notes.");
        assert_eq!(v["messages"].as_array().unwrap().len(), 1);
        assert_eq!(v["messages"][0]["role"], "user");
        assert_eq!(v["output_config"]["format"]["type"], "json_schema");
        assert_eq!(
            v["output_config"]["format"]["schema"]["required"][0],
            "tldr"
        );
    }

    #[test]
    fn needs_a_non_system_message() {
        let mut r = request();
        r.messages = vec![Message::system("only system")];
        assert!(matches!(provider().prepare(&r), Err(LlmError::Invalid(_))));
    }

    // Recorded shape (2026-09 docs), trimmed.
    const OK: &str = r#"{"id":"msg_01","type":"message","role":"assistant","model":"claude-x",
      "content":[{"type":"text","text":"{\"tldr\":[\"ok\"]}"}],
      "stop_reason":"end_turn","stop_sequence":null,
      "usage":{"input_tokens":200,"cache_creation_input_tokens":10,"cache_read_input_tokens":5,"output_tokens":41}}"#;

    #[test]
    fn parses_a_completion_with_usage() {
        let c = provider().parse(200, OK.as_bytes()).unwrap();
        assert_eq!(c.text, r#"{"tldr":["ok"]}"#);
        assert_eq!((c.tokens_in, c.tokens_out, c.truncated), (215, 41, false));
    }

    #[test]
    fn max_tokens_is_truncated() {
        let cut = OK.replace(
            r#""stop_reason":"end_turn""#,
            r#""stop_reason":"max_tokens""#,
        );
        assert!(provider().parse(200, cut.as_bytes()).unwrap().truncated);
    }

    #[test]
    fn refusal_is_an_error() {
        let r = r#"{"content":[{"type":"text","text":"I can't help."}],"stop_reason":"refusal","usage":{"input_tokens":3,"output_tokens":4}}"#;
        let e = provider().parse(200, r.as_bytes()).unwrap_err();
        assert!(
            matches!(&e, LlmError::Provider { message, .. } if message.contains("refused")),
            "{e:?}"
        );
    }

    #[test]
    fn error_body_shape() {
        let body =
            br#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        let e = provider().parse(529, body).unwrap_err();
        assert!(
            matches!(e, LlmError::Provider { status: 529, ref message } if message == "Overloaded")
        );
    }

    #[test]
    fn odd_shapes_are_provider_errors_not_panics() {
        for body in [
            "{}",
            r#"{"content":[]}"#,
            r#"{"content":[{"type":"tool_use"}]}"#,
            "null",
        ] {
            assert!(
                matches!(
                    provider().parse(200, body.as_bytes()),
                    Err(LlmError::Provider { .. })
                ),
                "{body}"
            );
        }
    }
}
