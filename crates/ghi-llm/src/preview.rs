// SPDX-License-Identifier: Apache-2.0
//! The send preview: exact payload, token count, cost estimate and retention note.
//!
//! Nothing goes to a cloud provider until the user has seen this and confirmed
//! its `sha256`: the hash of the exact request body, which is the body the
//! `ghi_net::CloudGrant` is then minted for.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cloud::{CloudProvider, Prepared};
use crate::{LlmError, Result, redact};

const BUILTIN_PRICES: &str = include_str!("../prices.toml");

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SendPreview {
    pub provider: String,
    pub model: String,
    pub host: String,
    /// The request body, byte for byte.
    pub payload: String,
    /// Hex SHA-256 of the body: what the user confirms.
    pub sha256: String,
    /// `(bytes + 2) / 3` input tokens: a deliberate over-estimate.
    pub tokens_est: u32,
    /// Input estimate plus `max_tokens` of output at the listed prices: an
    /// upper bound. `None` when the model has no listed price.
    pub cost_est_usd: Option<f64>,
    pub retention_note: String,
    /// Things in the text that still look like personal data.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Price {
    provider: String,
    model: String,
    /// USD per 1M input tokens.
    input: f64,
    /// USD per 1M output tokens.
    output: f64,
}

#[derive(Debug, Deserialize)]
struct PriceFile {
    #[serde(default)]
    model: Vec<Price>,
}

/// Per-1M-token prices by provider and model.
#[derive(Debug, Clone, PartialEq)]
pub struct Prices(Vec<Price>);

impl Prices {
    /// The table shipped in `prices.toml`.
    pub fn builtin() -> Prices {
        Prices::parse(BUILTIN_PRICES).expect("the built-in prices.toml is valid")
    }

    /// A user table in the same format (replaces the built-in one).
    pub fn from_file(path: &Path) -> Result<Prices> {
        Prices::parse(&std::fs::read_to_string(path)?)
    }

    fn parse(text: &str) -> Result<Prices> {
        let file: PriceFile =
            toml::from_str(text).map_err(|e| LlmError::Invalid(format!("prices file: {e}")))?;
        if let Some(p) = file.model.iter().find(|p| {
            !(p.input >= 0.0 && p.output >= 0.0 && p.input.is_finite() && p.output.is_finite())
        }) {
            return Err(LlmError::Invalid(format!(
                "prices file: bad price for {}",
                p.model
            )));
        }
        Ok(Prices(file.model))
    }

    /// `(input, output)` USD per 1M tokens. The longest listed model id that
    /// is `model` or a prefix of it wins.
    pub fn lookup(&self, provider: &str, model: &str) -> Option<(f64, f64)> {
        self.0
            .iter()
            .filter(|p| p.provider == provider && model.starts_with(&p.model))
            .max_by_key(|p| p.model.len())
            .map(|p| (p.input, p.output))
    }
}

/// `(bytes + 2) / 3`, the preview's token estimate for `bytes` of request.
pub fn estimate_tokens(bytes: usize) -> u32 {
    u32::try_from(bytes.div_ceil(3)).unwrap_or(u32::MAX)
}

/// What the user sees before a send.
pub fn preview(provider: &CloudProvider, prepared: &Prepared, prices: &Prices) -> SendPreview {
    let tokens_est = estimate_tokens(prepared.body.len());
    let json: Option<Value> = serde_json::from_slice(&prepared.body).ok();
    let max_out = json.as_ref().and_then(|v| {
        ["max_tokens", "max_completion_tokens"]
            .iter()
            .find_map(|k| v.get(*k).and_then(Value::as_u64))
    });
    let cost_est_usd = prices
        .lookup(provider.name(), &provider.model)
        .map(|(input, output)| {
            (f64::from(tokens_est) * input + max_out.unwrap_or(0) as f64 * output) / 1_000_000.0
        });
    let warnings = json
        .as_ref()
        .map(|v| redact::warnings(&message_text(v)))
        .unwrap_or_default();
    SendPreview {
        provider: provider.name().to_owned(),
        model: provider.model.clone(),
        host: prepared.host(),
        payload: String::from_utf8_lossy(&prepared.body).into_owned(),
        sha256: ghi_net::sha256_hex(&prepared.body),
        tokens_est,
        cost_est_usd,
        retention_note: retention_note(provider.name()),
        warnings,
    }
}

/// Every message string of a chat request (OpenAI `messages`, Anthropic
/// `system` and `messages`), joined: the text that carries transcript content.
fn message_text(v: &Value) -> String {
    let mut out = Vec::new();
    if let Some(s) = v.get("system").and_then(Value::as_str) {
        out.push(s);
    }
    if let Some(ms) = v.get("messages").and_then(Value::as_array) {
        out.extend(
            ms.iter()
                .filter_map(|m| m.get("content").and_then(Value::as_str)),
        );
    }
    out.join("\n")
}

fn retention_note(provider: &str) -> String {
    format!(
        "Text only, never audio. {provider} handles this text on its servers under the terms of \
         your account there; Ghira cannot see or control how long it is kept."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Message, Request};

    fn request(user: &str) -> Request {
        Request {
            messages: vec![Message::system("Write notes."), Message::user(user)],
            schema: Some(serde_json::json!({"type": "object"})),
            max_tokens: 1000,
            temperature: 0.2,
        }
    }

    fn prepared(name: &str, model: &str, user: &str) -> (CloudProvider, Prepared) {
        let p = CloudProvider::preset(name, model).unwrap();
        let prep = p.prepare(&request(user)).unwrap();
        (p, prep)
    }

    #[test]
    fn preview_shows_the_exact_bytes_and_their_hash() {
        let (p, prep) = prepared("openai", "gpt-4o-mini", "[s1] S1: hello <<EMAIL_1>>");
        let pv = preview(&p, &prep, &Prices::builtin());
        assert_eq!(pv.payload.as_bytes(), prep.body.as_slice());
        assert_eq!(pv.sha256, ghi_net::sha256_hex(&prep.body));
        assert_eq!(
            (pv.provider.as_str(), pv.host.as_str()),
            ("openai", "api.openai.com")
        );
        assert_eq!(pv.tokens_est as usize, prep.body.len().div_ceil(3));
        assert!(pv.retention_note.contains("never audio"));
        assert!(pv.warnings.is_empty(), "{:?}", pv.warnings);
        let json = serde_json::to_value(&pv).unwrap();
        assert_eq!(json["sha256"], pv.sha256.as_str());
    }

    #[test]
    fn cost_is_input_plus_max_output_at_listed_prices() {
        let (p, prep) = prepared("openai", "gpt-4o-mini", "hi");
        let pv = preview(&p, &prep, &Prices::builtin());
        let want = (f64::from(pv.tokens_est) * 0.15 + 1000.0 * 0.60) / 1e6;
        assert!((pv.cost_est_usd.unwrap() - want).abs() < 1e-12);
        // A dated snapshot uses the listed prefix; the longest prefix wins.
        let (p, prep) = prepared("anthropic", "claude-haiku-4-5-20251001", "hi");
        assert!(
            preview(&p, &prep, &Prices::builtin())
                .cost_est_usd
                .is_some()
        );
        let prices = Prices::builtin();
        assert_eq!(
            prices.lookup("openai", "gpt-4o-mini-2024-07-18"),
            Some((0.15, 0.60))
        );
        assert_eq!(
            prices.lookup("openai", "gpt-4o-2024-08-06"),
            Some((2.50, 10.00))
        );
        // Unknown model or provider: no estimate.
        let (p, prep) = prepared("gemini", "gemini-9-ultra", "hi");
        assert_eq!(preview(&p, &prep, &Prices::builtin()).cost_est_usd, None);
        assert_eq!(prices.lookup("anthropic", "gpt-4o"), None);
    }

    #[test]
    fn warnings_flag_leftover_personal_data() {
        let (p, prep) = prepared(
            "anthropic",
            "claude-haiku-4-5",
            "call me at 0912345678 or me@x.org",
        );
        let pv = preview(&p, &prep, &Prices::builtin());
        assert!(pv.warnings.len() >= 2, "{:?}", pv.warnings);
    }

    #[test]
    fn price_files() {
        let dir = std::env::temp_dir().join(format!("ghi-prices-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("prices.toml");
        std::fs::write(
            &f,
            "[[model]]\nprovider=\"openai\"\nmodel=\"x\"\ninput=1.0\noutput=2.0\n",
        )
        .unwrap();
        assert_eq!(
            Prices::from_file(&f).unwrap().lookup("openai", "x1"),
            Some((1.0, 2.0))
        );
        std::fs::write(
            &f,
            "[[model]]\nprovider=\"openai\"\nmodel=\"x\"\ninput=-1.0\noutput=2.0\n",
        )
        .unwrap();
        assert!(Prices::from_file(&f).is_err());
        std::fs::write(&f, "not toml [").unwrap();
        assert!(matches!(Prices::from_file(&f), Err(LlmError::Invalid(_))));
        assert!(matches!(
            Prices::from_file(&dir.join("missing")),
            Err(LlmError::Io(_))
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn estimate_rounds_up() {
        assert_eq!(
            (
                estimate_tokens(0),
                estimate_tokens(1),
                estimate_tokens(3),
                estimate_tokens(4)
            ),
            (0, 1, 1, 2)
        );
    }
}
