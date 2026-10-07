// SPDX-License-Identifier: Apache-2.0
//! OpenAI chat completions, also spoken by Gemini's compatible endpoint and
//! many self-hosted servers.

use serde_json::{Value, json};

use super::{Prepared, REASONING_MAX_TOKENS, Traits, excerpt};
use crate::{Completion, LlmError, Request, Result};

/// Name of the structured-output schema.
pub(super) const SCHEMA_NAME: &str = "ghira_output";
const OPENAI_HOST: &str = "https://api.openai.com/";

pub(super) fn prepare(base_url: &str, model: &str, req: &Request, t: Traits) -> Prepared {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|m| json!({"role": m.role, "content": m.content}))
        .collect();
    let mut body = json!({
        "model": model,
        "messages": messages,
    });
    if t.temperature {
        body["temperature"] = json!(req.temperature);
    }
    if let Some(effort) = t.effort {
        body["reasoning_effort"] = json!(effort);
    }
    // OpenAI's newer models reject `max_tokens`; other servers expect it.
    let limit = if base_url.starts_with(OPENAI_HOST) {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    let cap = if t.reasons {
        req.max_tokens.max(REASONING_MAX_TOKENS)
    } else {
        req.max_tokens
    };
    body[limit] = json!(cap);
    if let Some(schema) = &req.schema {
        body["response_format"] = json!({
            "type": "json_schema",
            "json_schema": {"name": SCHEMA_NAME, "schema": schema, "strict": true},
        });
    }
    Prepared {
        url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
        body: serde_json::to_vec(&body).expect("a JSON value serializes"),
    }
}

pub(super) fn parse(status: u16, json: &Value) -> Result<Completion> {
    let bad = |what: &str| LlmError::Provider {
        status,
        message: format!("unexpected answer: {what}"),
    };
    let choice = json
        .pointer("/choices/0")
        .ok_or_else(|| bad("no choices"))?;
    let message = choice.get("message").ok_or_else(|| bad("no message"))?;
    if let Some(refusal) = message.get("refusal").and_then(Value::as_str)
        && !refusal.is_empty()
    {
        return Err(LlmError::Provider {
            status,
            message: format!("the model refused: {}", excerpt(refusal)),
        });
    }
    let finish = choice.get("finish_reason").and_then(Value::as_str);
    if finish == Some("content_filter") {
        return Err(LlmError::Provider {
            status,
            message: "the provider's content filter stopped the answer".into(),
        });
    }
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("no content"))?;
    let count = |key: &str| {
        json.pointer(&format!("/usage/{key}"))
            .and_then(Value::as_u64)
            .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
    };
    Ok(Completion {
        text: text.to_owned(),
        tokens_in: count("prompt_tokens"),
        tokens_out: count("completion_tokens"),
        truncated: finish == Some("length"),
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::request;
    use super::super::{CloudProvider, LlmError};
    use serde_json::Value;

    fn body(name: &str) -> Value {
        let model = if name == "gemini" {
            "gemini-2.0-flash"
        } else {
            "gpt-4.1-mini"
        };
        let p = CloudProvider::preset(name, model).unwrap();
        serde_json::from_slice(&p.prepare(&request()).unwrap().body).unwrap()
    }

    #[test]
    fn openai_request_shape() {
        let p = CloudProvider::preset("openai", "gpt-4.1-mini").unwrap();
        let prepared = p.prepare(&request()).unwrap();
        assert_eq!(prepared.url, "https://api.openai.com/v1/chat/completions");
        let v = body("openai");
        assert_eq!(v["model"], "gpt-4.1-mini");
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][1]["role"], "user");
        assert_eq!(v["max_completion_tokens"], 2048);
        assert!(v.get("max_tokens").is_none());
        assert_eq!(v["response_format"]["type"], "json_schema");
        assert_eq!(v["response_format"]["json_schema"]["name"], "ghira_output");
        assert_eq!(v["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            v["response_format"]["json_schema"]["schema"]["required"][0],
            "tldr"
        );
        assert!(v["temperature"].as_f64().is_some());
        assert!(v.get("reasoning_effort").is_none());
    }

    #[test]
    fn reasoning_models_get_low_effort_room_and_no_temperature() {
        for (name, model, limit) in [
            ("openai", "gpt-6.1-sol", "max_completion_tokens"),
            ("openai", "o4-mini", "max_completion_tokens"),
            ("gemini", "gemini-3.8-flash", "max_tokens"),
        ] {
            let p = CloudProvider::preset(name, model).unwrap();
            let v: Value = serde_json::from_slice(&p.prepare(&request()).unwrap().body).unwrap();
            assert!(v.get("temperature").is_none(), "{model}: {v}");
            assert_eq!(v["reasoning_effort"], "low", "{model}");
            assert_eq!(v[limit], 16_384, "{model}");
            assert_eq!(v["response_format"]["type"], "json_schema");
        }
        // Another OpenAI-compatible server keeps what it always got.
        let c = CloudProvider::openai_compat("https://llm.example.com/v1", "gpt-6.1-sol").unwrap();
        let v: Value = serde_json::from_slice(&c.prepare(&request()).unwrap().body).unwrap();
        assert!(v["temperature"].as_f64().is_some());
        assert_eq!(v["max_tokens"], 2048);
    }

    #[test]
    fn gemini_and_compat_shape() {
        let g = CloudProvider::preset("gemini", "gemini-x").unwrap();
        assert_eq!(
            g.prepare(&request()).unwrap().url,
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
        let v = body("gemini");
        assert_eq!(v["max_tokens"], 2048);
        assert_eq!(v["response_format"]["type"], "json_schema");
        let c = CloudProvider::openai_compat("https://llm.example.com/v1/", "m").unwrap();
        assert_eq!(
            c.prepare(&request()).unwrap().url,
            "https://llm.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn no_schema_means_no_response_format() {
        let mut r = request();
        r.schema = None;
        let p = CloudProvider::preset("openai", "m").unwrap();
        let v: Value = serde_json::from_slice(&p.prepare(&r).unwrap().body).unwrap();
        assert!(v.get("response_format").is_none());
    }

    // Recorded shapes (2026-09 docs), trimmed.
    const OK: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","model":"gpt-x",
      "choices":[{"index":0,"message":{"role":"assistant","content":"{\"tldr\":[\"ok\"]}","refusal":null},
      "finish_reason":"stop"}],
      "usage":{"prompt_tokens":120,"completion_tokens":33,"total_tokens":153,
      "prompt_tokens_details":{"cached_tokens":0},"completion_tokens_details":{"reasoning_tokens":0}}}"#;

    #[test]
    fn parses_a_completion_with_usage() {
        let p = CloudProvider::preset("openai", "m").unwrap();
        let c = p.parse(200, OK.as_bytes()).unwrap();
        assert_eq!(c.text, r#"{"tldr":["ok"]}"#);
        assert_eq!((c.tokens_in, c.tokens_out, c.truncated), (120, 33, false));
        // Gemini's compatible endpoint uses the same shape.
        let g = CloudProvider::preset("gemini", "m").unwrap();
        assert_eq!(g.parse(200, OK.as_bytes()).unwrap(), c);
    }

    #[test]
    fn length_finish_is_truncated() {
        let cut = OK.replace(r#""finish_reason":"stop""#, r#""finish_reason":"length""#);
        let p = CloudProvider::preset("openai", "m").unwrap();
        assert!(p.parse(200, cut.as_bytes()).unwrap().truncated);
    }

    #[test]
    fn refusal_and_content_filter_are_errors() {
        let p = CloudProvider::preset("openai", "m").unwrap();
        let refused = r#"{"choices":[{"message":{"role":"assistant","content":null,"refusal":"I can't help with that."},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":6}}"#;
        let e = p.parse(200, refused.as_bytes()).unwrap_err();
        assert!(
            matches!(&e, LlmError::Provider { message, .. } if message.contains("refused")),
            "{e:?}"
        );
        let filtered = r#"{"choices":[{"message":{"role":"assistant","content":"x"},"finish_reason":"content_filter"}]}"#;
        assert!(matches!(
            p.parse(200, filtered.as_bytes()),
            Err(LlmError::Provider { .. })
        ));
    }

    #[test]
    fn odd_shapes_are_provider_errors_not_panics() {
        let p = CloudProvider::preset("openai", "m").unwrap();
        for body in [
            "{}",
            r#"{"choices":[]}"#,
            r#"{"choices":[{"message":{}}]}"#,
            "[]",
            "null",
            "not json",
        ] {
            assert!(
                matches!(
                    p.parse(200, body.as_bytes()),
                    Err(LlmError::Provider { .. })
                ),
                "{body}"
            );
        }
    }
}
