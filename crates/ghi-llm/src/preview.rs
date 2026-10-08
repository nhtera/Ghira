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
    /// Input tokens, a deliberate over-estimate ([`estimate_request_tokens`]).
    pub tokens_est: u32,
    /// The likely cost: the input estimate plus a typical answer
    /// ([`TYPICAL_OUTPUT_TOKENS`], at most `max_tokens`) at the listed
    /// prices. `None` when the model has no listed price.
    pub cost_est_usd: Option<f64>,
    /// The most it can cost: the input estimate plus all of `max_tokens`
    /// (16k for a reasoning model) of output.
    pub cost_max_usd: Option<f64>,
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
    /// An introductory price: `input`/`output` apply up to this day (UTC,
    /// `YYYY-MM-DD`), `then_input`/`then_output` after it.
    #[serde(default)]
    until: Option<String>,
    #[serde(default)]
    then_input: Option<f64>,
    #[serde(default)]
    then_output: Option<f64>,
    /// A dearer price for a long prompt: over `over_tokens` input tokens the
    /// request pays `over_input`/`over_output` (Claude Haiku 5.5 over 100k).
    #[serde(default)]
    over_tokens: Option<u64>,
    #[serde(default)]
    over_input: Option<f64>,
    #[serde(default)]
    over_output: Option<f64>,
}

/// A model id (or id prefix) the provider has shut down or deprecated, or
/// that a newer listed model replaces (`successor`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Retired {
    provider: String,
    model: String,
    #[serde(default)]
    successor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PriceFile {
    #[serde(default)]
    model: Vec<Price>,
    #[serde(default)]
    retired: Vec<Retired>,
}

/// Per-1M-token prices by provider and model, and the retired models.
#[derive(Debug, Clone, PartialEq)]
pub struct Prices {
    models: Vec<Price>,
    retired: Vec<Retired>,
}

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
        let ok = |v: f64| v >= 0.0 && v.is_finite();
        if let Some(p) = file.model.iter().find(|p| {
            ![p.input, p.output]
                .into_iter()
                .chain(
                    [p.then_input, p.then_output, p.over_input, p.over_output]
                        .into_iter()
                        .flatten(),
                )
                .all(ok)
        }) {
            return Err(LlmError::Invalid(format!(
                "prices file: bad price for {}",
                p.model
            )));
        }
        Ok(Prices {
            models: file.model,
            retired: file.retired,
        })
    }

    /// Every listed `(provider, model)`, in file order (the model menus).
    pub fn models(&self) -> Vec<(String, String)> {
        self.models
            .iter()
            .map(|p| (p.provider.clone(), p.model.clone()))
            .collect()
    }

    /// `(input, output)` USD per 1M tokens. The longest listed model id that
    /// is `model` or a prefix of it wins.
    pub fn lookup(&self, provider: &str, model: &str) -> Option<(f64, f64)> {
        self.lookup_on(provider, model, &today_utc())
    }

    /// [`Prices::lookup`] on `day` (`YYYY-MM-DD`): an introductory price
    /// gives way to its regular one after its `until` day.
    pub fn lookup_on(&self, provider: &str, model: &str, day: &str) -> Option<(f64, f64)> {
        self.lookup_for(provider, model, day, 0)
    }

    /// [`Prices::lookup_on`] for a request of `prompt_tokens` input tokens:
    /// a long prompt pays the model's `over_*` price when it has one.
    pub fn lookup_for(
        &self,
        provider: &str,
        model: &str,
        day: &str,
        prompt_tokens: u64,
    ) -> Option<(f64, f64)> {
        self.models
            .iter()
            .filter(|p| p.provider == provider && model.starts_with(&p.model))
            .max_by_key(|p| p.model.len())
            .map(|p| {
                if let (Some(over), Some(i), Some(o)) = (p.over_tokens, p.over_input, p.over_output)
                    && prompt_tokens > over
                {
                    return (i, o);
                }
                match (&p.until, p.then_input, p.then_output) {
                    (Some(until), Some(i), Some(o)) if day > until.as_str() => (i, o),
                    _ => (p.input, p.output),
                }
            })
    }
}

impl Prices {
    /// `model`, or when it is retired (`[[retired]]`, by prefix) its
    /// `successor`, else the provider's first listed model (its menu
    /// default): a saved choice that the provider no longer serves, or that a
    /// newer model replaces, moves on instead of failing at send time.
    pub fn current_model(&self, provider: &str, model: &str) -> String {
        let retired = self
            .retired
            .iter()
            .find(|r| r.provider == provider && model.trim().starts_with(&r.model));
        let first = self.models.iter().find(|p| p.provider == provider);
        match (retired, first) {
            (
                Some(Retired {
                    successor: Some(s), ..
                }),
                _,
            ) => s.clone(),
            (Some(_), Some(p)) => p.model.clone(),
            _ => model.to_owned(),
        }
    }
}

/// Output tokens of a typical notes answer, for the likely cost. Live sends
/// of 2026-10-09 (all menu models, a 5-min meeting and a short VI one) wrote
/// 355-950 without thinking; a model that thinks first wrote up to ~2,500
/// (claude-haiku-5-5), thinking included, so it is counted with more.
pub const TYPICAL_OUTPUT_TOKENS: u64 = 1_500;
pub const TYPICAL_OUTPUT_TOKENS_THINKING: u64 = 3_000;

/// Today in UTC as `YYYY-MM-DD` (for introductory prices).
fn today_utc() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `(bytes + 2) / 3`, the preview's token estimate for `bytes` of request.
pub fn estimate_tokens(bytes: usize) -> u32 {
    u32::try_from(bytes.div_ceil(3)).unwrap_or(u32::MAX)
}

/// The input tokens a provider will bill for a request body of `bytes`, an
/// over-estimate. Measured live on 2026-10-09 (notes requests, EN and VI,
/// 2 to 92 minutes): OpenAI and Gemini billed 0.6-0.9x of `bytes / 3`;
/// Claude (its tokenizer since 4.7, plus ~290 tokens of tool-use prompt)
/// billed 1.45-1.6x, about `bytes / 2` (0.48-0.51 tokens a byte), so it is
/// counted as `0.55 * bytes + 300`.
pub fn estimate_request_tokens(provider: &str, bytes: usize) -> u32 {
    match provider {
        "anthropic" => u32::try_from((bytes as u64 * 11).div_ceil(20) + 300).unwrap_or(u32::MAX),
        _ => estimate_tokens(bytes),
    }
}

/// What the user sees before a send.
pub fn preview(provider: &CloudProvider, prepared: &Prepared, prices: &Prices) -> SendPreview {
    let tokens_est = estimate_request_tokens(provider.name(), prepared.body.len());
    let json: Option<Value> = serde_json::from_slice(&prepared.body).ok();
    let max_out = json.as_ref().and_then(|v| {
        ["max_tokens", "max_completion_tokens"]
            .iter()
            .find_map(|k| v.get(*k).and_then(Value::as_u64))
    });
    let price = prices.lookup_for(
        provider.name(),
        &provider.model,
        &today_utc(),
        u64::from(tokens_est),
    );
    let cost = |out: u64| {
        price.map(|(input, output)| {
            (f64::from(tokens_est) * input + out as f64 * output) / 1_000_000.0
        })
    };
    let answer = if crate::cloud::traits(provider.name(), &provider.model).reasons {
        TYPICAL_OUTPUT_TOKENS_THINKING
    } else {
        TYPICAL_OUTPUT_TOKENS
    };
    let typical = max_out.map_or(answer, |m| m.min(answer));
    let cost_est_usd = cost(typical);
    let cost_max_usd = cost(max_out.unwrap_or(typical));
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
        cost_max_usd,
        retention_note: retention_note(provider.name()),
        warnings,
    }
}

/// The text of the user-role messages of a request body (OpenAI and
/// Anthropic `messages`, Gemini `contents`), joined by newlines: the
/// transcript part, never the system prompt. An unknown shape gives "".
/// Mirrors `userText` in the desktop's cloud sheet.
pub fn user_text(payload: &str) -> String {
    fn text_of(v: Option<&Value>) -> Option<String> {
        match v? {
            Value::String(s) => Some(s.clone()),
            Value::Array(parts) => Some(
                parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        }
    }
    let Ok(v) = serde_json::from_str::<Value>(payload) else {
        return String::new();
    };
    let mut out = Vec::new();
    if let Some(ms) = v.get("messages").and_then(Value::as_array) {
        for m in ms {
            if m.get("role").and_then(Value::as_str) == Some("user") {
                out.extend(text_of(m.get("content")));
            }
        }
    }
    if let Some(cs) = v.get("contents").and_then(Value::as_array) {
        for c in cs {
            if c.get("role").is_some_and(|r| r.as_str() != Some("user")) {
                continue;
            }
            out.extend(text_of(c.get("parts")));
        }
    }
    out.retain(|t| !t.is_empty());
    out.join("\n")
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
    #[test]
    fn user_text_is_the_user_messages_only() {
        let openai = r#"{"messages":[{"role":"system","content":"RULES"},{"role":"user","content":"hello"}]}"#;
        assert_eq!(super::user_text(openai), "hello");
        let anthropic = r#"{"system":"RULES","messages":[{"role":"user","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}]}"#;
        assert_eq!(super::user_text(anthropic), "a\nb");
        let gemini =
            r#"{"contents":[{"parts":[{"text":"hi"}]},{"role":"model","parts":[{"text":"no"}]}]}"#;
        assert_eq!(super::user_text(gemini), "hi");
        assert_eq!(super::user_text("nope"), "");
        assert_eq!(super::user_text("{}"), "");
    }

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
    fn an_introductory_price_ends_on_its_day() {
        let prices = Prices::builtin();
        assert_eq!(
            prices.lookup_on("gemini", "gemini-3.8-flash", "2026-12-31"),
            Some((0.75, 3.75))
        );
        assert_eq!(
            prices.lookup_on("gemini", "gemini-3.8-flash", "2027-01-01"),
            Some((1.50, 7.50))
        );
        assert_eq!(
            prices.lookup_on("anthropic", "claude-haiku-5-5", "2027-06-01"),
            Some((0.10, 0.50))
        );
        // Claude Haiku 5.5 is dearer for a prompt over 100k tokens.
        let day = "2026-10-09";
        assert_eq!(
            prices.lookup_for("anthropic", "claude-haiku-5-5", day, 100_000),
            Some((0.10, 0.50))
        );
        assert_eq!(
            prices.lookup_for("anthropic", "claude-haiku-5-5", day, 100_001),
            Some((0.50, 2.50))
        );
        assert_eq!(
            prices.lookup_for("anthropic", "claude-sonnet-5-5", day, 900_000),
            Some((2.00, 10.00))
        );
        let t = today_utc();
        assert!(t.len() == 10 && t.starts_with("20"), "{t}");
    }

    #[test]
    fn cost_is_input_plus_a_typical_answer_and_at_most_max_output() {
        // max_tokens 1000 is under a typical answer: both are input + 1000.
        // (An older model, priced by a user file: no longer in the menus.)
        let older = Prices::parse(
            "[[model]]\nprovider=\"openai\"\nmodel=\"gpt-4.1-mini\"\ninput=0.4\noutput=1.6\n",
        )
        .unwrap();
        let (p, prep) = prepared("openai", "gpt-4.1-mini", "hi");
        let pv = preview(&p, &prep, &older);
        let want = (f64::from(pv.tokens_est) * 0.40 + 1000.0 * 1.60) / 1e6;
        assert!((pv.cost_est_usd.unwrap() - want).abs() < 1e-12);
        assert!((pv.cost_max_usd.unwrap() - want).abs() < 1e-12);
        // A dated snapshot uses the listed prefix; the longest prefix wins.
        let prices = Prices::parse(
            "[[model]]\nprovider=\"openai\"\nmodel=\"gpt-6\"\ninput=1.0\noutput=2.0\n\
             [[model]]\nprovider=\"openai\"\nmodel=\"gpt-6-luna\"\ninput=3.0\noutput=4.0\n",
        )
        .unwrap();
        assert_eq!(
            prices.lookup("openai", "gpt-6-luna-2026-09-15"),
            Some((3.0, 4.0))
        );
        assert_eq!(prices.lookup("openai", "gpt-6-astra"), Some((1.0, 2.0)));
        let prices = Prices::builtin();
        assert_eq!(prices.lookup("openai", "gpt-6.1-sol"), Some((2.00, 10.00)));
        // A reasoning model: likely cost with a typical answer, at most its
        // whole room to think (the one that was shown before, ~14x too high).
        let (p, prep) = prepared("anthropic", "claude-sonnet-5-5", "hi");
        let pv = preview(&p, &prep, &prices);
        let likely = (f64::from(pv.tokens_est) * 2.0 + 3_000.0 * 10.0) / 1e6;
        let most = (f64::from(pv.tokens_est) * 2.0 + 16_384.0 * 10.0) / 1e6;
        assert!((pv.cost_est_usd.unwrap() - likely).abs() < 1e-12);
        assert!((pv.cost_max_usd.unwrap() - most).abs() < 1e-12);
        // Unknown model or provider: no estimate.
        let (p, prep) = prepared("gemini", "gemini-9-ultra", "hi");
        assert_eq!(preview(&p, &prep, &Prices::builtin()).cost_est_usd, None);
        assert_eq!(prices.lookup("anthropic", "gpt-6-luna"), None);
    }

    #[test]
    fn the_menus_list_current_models_and_retired_ones_move_on() {
        let prices = Prices::builtin();
        let first = |provider: &str| {
            prices
                .models()
                .into_iter()
                .find(|(p, _)| p == provider)
                .map(|(_, m)| m)
        };
        assert_eq!(first("anthropic").as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(first("openai").as_deref(), Some("gpt-6.1-sol"));
        assert_eq!(first("gemini").as_deref(), Some("gemini-3.8-flash"));
        for (provider, model) in prices.models() {
            assert_eq!(prices.current_model(&provider, &model), model);
            assert!(prices.lookup(&provider, &model).is_some());
        }
        assert_eq!(
            prices.current_model("anthropic", "claude-sonnet-4-5-20250929"),
            "claude-sonnet-5-5"
        );
        assert_eq!(
            prices.current_model("gemini", "gemini-2.5-flash"),
            "gemini-3.8-flash"
        );
        // Replaced by a newer model of the same tier: moves to it.
        for (provider, old, new) in [
            ("anthropic", "claude-haiku-4-5", "claude-haiku-5-5"),
            ("anthropic", "claude-haiku-4-5-20251001", "claude-haiku-5-5"),
            ("openai", "gpt-4.1-mini", "gpt-6-luna"),
            ("openai", "gpt-6-sol", "gpt-6.1-sol"),
            ("gemini", "gemini-3.1-flash-lite", "gemini-3.5-flash-lite"),
        ] {
            assert_eq!(prices.current_model(provider, old), new, "{old}");
        }
        // Every successor is a listed model of its provider.
        for r in &prices.retired {
            if let Some(s) = &r.successor {
                assert!(
                    prices.models().contains(&(r.provider.clone(), s.clone())),
                    "{s}"
                );
            }
        }
        // Still served and never in the menu: kept.
        assert_eq!(prices.current_model("openai", "gpt-4o"), "gpt-4o");
        assert_eq!(prices.current_model("anthropic", ""), "");
    }

    #[test]
    fn warnings_flag_leftover_personal_data() {
        let (p, prep) = prepared(
            "anthropic",
            "claude-haiku-5-5",
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
        // Claude bills more tokens a byte: its estimate stays above the
        // counts it billed live (4,960 bytes: 2,401; 86,632 bytes: 44,171).
        assert_eq!(estimate_request_tokens("openai", 4_960), 1_654);
        assert!(estimate_request_tokens("anthropic", 4_960) >= 2_401);
        assert!(estimate_request_tokens("anthropic", 86_632) >= 44_171);
        let (p, prep) = prepared("anthropic", "claude-haiku-5-5", "hi");
        assert_eq!(
            preview(&p, &prep, &Prices::builtin()).tokens_est,
            estimate_request_tokens("anthropic", prep.body.len())
        );
    }
}
