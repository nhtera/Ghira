// SPDX-License-Identifier: Apache-2.0
//! Cloud providers (OpenAI-compatible, Anthropic, Gemini): prepare → send preview → confirmed send through `ghi-net`.
//!
//! A provider is never an [`crate::Llm`]. The caller builds the request
//! (redacted transcript, `notes::request(.., Dialect::Cloud)`), calls
//! [`CloudProvider::prepare`] to get the exact bytes, shows them in a
//! [`crate::preview::SendPreview`], and only after the user confirmed the
//! SHA-256 mints a `ghi_net::CloudGrant` for those bytes. [`CloudProvider::send`]
//! posts them and parses the answer; the API key appears only in the headers
//! built by [`CloudProvider::headers`], never in the body, the URL or an error.
//!
//! Providers are deliberately thin: the JSON schema is passed through as the
//! provider's structured-output format, and the answer is still validated by
//! the engine like local output. Schemas for the cloud dialect must stay in the
//! common subset (objects with `additionalProperties: false`, every property
//! required, no numeric or string-length constraints).

mod anthropic;
mod openai;

use std::sync::LazyLock;
use std::time::Duration;

use ghi_net::{CloudGrant, Headers, NetError, Secret};
use regex::Regex;

use crate::{Completion, LlmError, Request, Result};

pub const OPENAI_BASE: &str = "https://api.openai.com/v1";
pub const ANTHROPIC_BASE: &str = "https://api.anthropic.com";
/// Gemini's OpenAI-compatible endpoint; the key goes in `Authorization`,
/// never in the URL.
pub const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/openai/";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Provider error text kept in an error message.
const MAX_ERROR_CHARS: usize = 300;
/// Pauses before the 2nd and 3rd attempt.
const BACKOFF: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(6)];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudKind {
    /// `{base_url}/chat/completions` (OpenAI and compatible servers).
    OpenAiCompat {
        base_url: String,
    },
    Anthropic,
    /// A preset of the OpenAI-compatible endpoint at [`GEMINI_BASE`].
    Gemini,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudProvider {
    pub kind: CloudKind,
    pub model: String,
}

/// The exact request: what the preview shows, what the grant binds, what is
/// sent. Holds no key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub url: String,
    pub body: Vec<u8>,
}

impl Prepared {
    /// The host the request goes to.
    pub fn host(&self) -> String {
        let rest = self
            .url
            .split_once("://")
            .map_or(self.url.as_str(), |x| x.1);
        let authority = rest.split(['/', '?']).next().unwrap_or("");
        let host = authority.rsplit_once('@').map_or(authority, |x| x.1);
        match host.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or("").to_owned(),
            None => host.split(':').next().unwrap_or("").to_owned(),
        }
        .to_ascii_lowercase()
    }
}

impl CloudProvider {
    /// `openai`, `anthropic` or `gemini` with `model`.
    pub fn preset(name: &str, model: &str) -> Result<CloudProvider> {
        let kind = match name.to_ascii_lowercase().as_str() {
            "openai" => CloudKind::OpenAiCompat {
                base_url: OPENAI_BASE.into(),
            },
            "anthropic" => CloudKind::Anthropic,
            "gemini" => CloudKind::Gemini,
            _ => {
                return Err(LlmError::Invalid(format!(
                    "unknown cloud provider {name:?} (openai, anthropic, gemini)"
                )));
            }
        };
        CloudProvider::new(kind, model)
    }

    /// Any OpenAI-compatible server reachable over https.
    pub fn openai_compat(base_url: &str, model: &str) -> Result<CloudProvider> {
        if !base_url.starts_with("https://") {
            return Err(LlmError::Invalid(
                "a provider base URL must be https".into(),
            ));
        }
        // No userinfo, query, fragment, backslash or spaces: the host the send
        // preview shows must be the host the request goes to.
        let rest = &base_url["https://".len()..];
        if rest.is_empty()
            || rest.starts_with('/')
            || rest
                .chars()
                .any(|c| matches!(c, '@' | '#' | '?' | '\\') || c.is_whitespace() || c.is_control())
        {
            return Err(LlmError::Invalid(
                "a provider base URL is https://host[:port]/path, nothing else".into(),
            ));
        }
        CloudProvider::new(
            CloudKind::OpenAiCompat {
                base_url: base_url.to_owned(),
            },
            model,
        )
    }

    fn new(kind: CloudKind, model: &str) -> Result<CloudProvider> {
        if model.trim().is_empty() {
            return Err(LlmError::Invalid("a cloud model name is required".into()));
        }
        Ok(CloudProvider {
            kind,
            model: model.trim().to_owned(),
        })
    }

    /// `openai`, `anthropic`, `gemini`, or `openai-compat` for other base URLs.
    pub fn name(&self) -> &'static str {
        match &self.kind {
            CloudKind::OpenAiCompat { base_url } if base_url == OPENAI_BASE => "openai",
            CloudKind::OpenAiCompat { .. } => "openai-compat",
            CloudKind::Anthropic => "anthropic",
            CloudKind::Gemini => "gemini",
        }
    }

    /// The request as exact bytes (same input, same bytes).
    pub fn prepare(&self, req: &Request) -> Result<Prepared> {
        let t = traits(self.name(), &self.model);
        match &self.kind {
            CloudKind::OpenAiCompat { base_url } => {
                Ok(openai::prepare(base_url, &self.model, req, t))
            }
            CloudKind::Gemini => Ok(openai::prepare(GEMINI_BASE, &self.model, req, t)),
            CloudKind::Anthropic => anthropic::prepare(ANTHROPIC_BASE, &self.model, req, t),
        }
    }

    /// Request headers for `key`. The only place a key is used.
    pub fn headers(&self, key: &Secret) -> Headers {
        let h = Headers::new().plain("content-type", "application/json");
        match &self.kind {
            CloudKind::Anthropic => h
                .plain("anthropic-version", ANTHROPIC_VERSION)
                .secret("x-api-key", key.clone()),
            _ => h.secret(
                "authorization",
                Secret::new(format!("Bearer {}", key.expose())),
            ),
        }
    }

    /// Turns an HTTP answer into a [`Completion`]. A provider error, a refusal
    /// or an unexpected shape is an error; a cut-off answer is `truncated`.
    pub fn parse(&self, status: u16, body: &[u8]) -> Result<Completion> {
        if !(200..300).contains(&status) {
            return Err(LlmError::Provider {
                status,
                message: error_message(body),
            });
        }
        let json: serde_json::Value =
            serde_json::from_slice(body).map_err(|_| LlmError::Provider {
                status,
                message: "the answer is not JSON".into(),
            })?;
        match &self.kind {
            CloudKind::Anthropic => anthropic::parse(status, &json),
            _ => openai::parse(status, &json),
        }
    }

    /// Sends `prepared` under `grant` and parses the answer. Retries 429, 5xx
    /// and transport failures (same bytes, while the grant has attempts).
    pub fn send(
        &self,
        prepared: &Prepared,
        grant: &mut CloudGrant,
        key: &Secret,
    ) -> Result<Completion> {
        self.send_with(prepared, grant, key, &BACKOFF)
    }

    fn send_with(
        &self,
        prepared: &Prepared,
        grant: &mut CloudGrant,
        key: &Secret,
        backoff: &[Duration],
    ) -> Result<Completion> {
        let headers = self.headers(key);
        log::info!(
            "cloud send provider={} model={} bytes={}",
            self.name(),
            self.model,
            prepared.body.len()
        );
        let mut attempt = 0;
        loop {
            let last = grant.attempts_left() <= 1;
            match ghi_net::send(grant, &prepared.url, &headers, &prepared.body) {
                Ok(r) if (r.status == 429 || r.status >= 500) && !last => {}
                Ok(r) => {
                    return self.parse(r.status, &r.body).map_err(|e| scrub(e, key));
                }
                Err(e) if e.is_retryable() && !last => {}
                Err(e) => {
                    let e = map_net(e);
                    let verdict = if matches!(e, LlmError::Denied(_)) {
                        "refused"
                    } else {
                        "failed"
                    };
                    log::warn!("cloud send {verdict} provider={}", self.name());
                    return Err(e);
                }
            }
            std::thread::sleep(backoff.get(attempt).copied().unwrap_or_default());
            attempt += 1;
        }
    }
}

fn map_net(e: NetError) -> LlmError {
    match e {
        NetError::Denied(_) | NetError::Mismatch(_) | NetError::Expired | NetError::Exhausted => {
            LlmError::Denied(e.to_string())
        }
        _ => LlmError::Net(e.to_string()),
    }
}

/// Provider error text, from the JSON `error.message` if there is one, cut to
/// [`MAX_ERROR_CHARS`] and with key-looking tokens masked.
fn error_message(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let from_json = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            // Gemini's compatible endpoint sometimes wraps the error in an array.
            let v = v.get(0).cloned().unwrap_or(v);
            let e = v.get("error")?;
            e.get("message")
                .and_then(|m| m.as_str())
                .or_else(|| e.as_str())
                .map(str::to_owned)
        });
    let msg = from_json.unwrap_or_else(|| text.into_owned());
    mask_keys(&one_line(&msg, MAX_ERROR_CHARS))
}

fn one_line(s: &str, max_chars: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max_chars {
        return flat;
    }
    let cut: String = flat.chars().take(max_chars).collect();
    format!("{cut}...")
}

/// What a model accepts, read from its id. Newer models reject sampling
/// settings and think before they answer; their thinking counts against the
/// output cap. An id this doesn't know is treated as the newer kind on the
/// big three (omitting `temperature` is always accepted) and as before on
/// other OpenAI-compatible servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Traits {
    /// Takes `temperature`.
    pub temperature: bool,
    /// Thinks before answering: the cap is raised to [`REASONING_MAX_TOKENS`].
    pub reasons: bool,
    /// The reasoning effort to ask for (OpenAI-style `reasoning_effort`).
    pub effort: Option<&'static str>,
}

/// Output room for a model that thinks first: notes in JSON plus its thinking.
/// Unused room is not billed; the send preview's cost stays an upper bound.
pub(crate) const REASONING_MAX_TOKENS: u32 = 16_384;

const OLDER: Traits = Traits {
    temperature: true,
    reasons: false,
    effort: None,
};

/// `(major, minor)` after `prefix` in ids like `claude-sonnet-4-5`,
/// `gpt-4.1-mini` or `gemini-3.5-flash`; a dated suffix is not a minor.
fn version(id: &str, prefix: &Regex) -> Option<(u32, u32)> {
    let c = prefix.captures(id)?;
    let major = c.get(1)?.as_str().parse().ok()?;
    let minor = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
    Some((major, minor))
}

static CLAUDE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^claude-[a-z]+-(\d+)(?:-(\d{1,2}))?(?:$|-)").expect("valid regex")
});
static CLAUDE_OLD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^claude-(\d+)(?:-(\d{1,2}))?-").expect("valid regex"));
static GPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^gpt-(\d+)(?:\.(\d+))?").expect("valid regex"));
static O_SERIES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^o\d").expect("valid regex"));
static GEMINI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:models/)?gemini-(\d+)(?:\.(\d+))?").expect("valid regex"));

/// [`Traits`] of `model` at provider `name` ([`CloudProvider::name`]).
pub(crate) fn traits(name: &str, model: &str) -> Traits {
    let id = model.trim().to_ascii_lowercase();
    let newer = Traits {
        temperature: false,
        reasons: true,
        effort: None,
    };
    match name {
        // Claude 4.7 and later answer 400 to a non-default temperature, and
        // their adaptive thinking spends the same cap.
        "anthropic" => match version(&id, &CLAUDE).or_else(|| version(&id, &CLAUDE_OLD)) {
            Some(v) if v < (4, 7) => OLDER,
            _ => newer,
        },
        // GPT-5 and later, and the o-series, reason (default effort medium)
        // and take no temperature; GPT-4.x is the older kind.
        "openai" => match version(&id, &GPT) {
            Some((major, _)) if major < 5 => OLDER,
            Some(_) => Traits {
                effort: Some("low"),
                ..newer
            },
            None if O_SERIES.is_match(&id) => Traits {
                effort: Some("low"),
                ..newer
            },
            None => newer,
        },
        // Gemini 3 always thinks (`low` is its lightest effort through the
        // OpenAI layer) and is tuned for its default temperature.
        "gemini" => match version(&id, &GEMINI) {
            Some((major, _)) if major < 3 => OLDER,
            _ => Traits {
                effort: Some("low"),
                ..newer
            },
        },
        _ => OLDER,
    }
}

static KEY_LIKE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(sk-[A-Za-z0-9_*.\-]{6,}|AIza[0-9A-Za-z_\-]{16,}|Bearer\s+\S+)")
        .expect("valid regex")
});

fn mask_keys(s: &str) -> String {
    KEY_LIKE.replace_all(s, "[redacted]").into_owned()
}

/// Belt and braces: a provider error must not echo the key we sent.
fn scrub(e: LlmError, key: &Secret) -> LlmError {
    match e {
        LlmError::Provider { status, message } if !key.expose().is_empty() => LlmError::Provider {
            status,
            message: message.replace(key.expose(), "[redacted]"),
        },
        e => e,
    }
}

/// A short, single-line excerpt of model text for error messages.
pub(crate) fn excerpt(s: &str) -> String {
    mask_keys(&one_line(s, MAX_ERROR_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;
    use ghi_net::{CloudGrant, MeetingGate, NetPolicy};

    pub(crate) fn request() -> Request {
        Request {
            messages: vec![
                Message::system("You write meeting notes."),
                Message::user("[s1] S1: hello <<EMAIL_1>>"),
            ],
            schema: Some(serde_json::json!({
                "type": "object",
                "properties": {"tldr": {"type": "array", "items": {"type": "string"}}},
                "required": ["tldr"],
                "additionalProperties": false,
            })),
            max_tokens: 2048,
            temperature: 0.2,
        }
    }

    const KEY: &str = "test-key-not-real";

    #[test]
    fn traits_follow_the_model_generation() {
        let t = |p, m| {
            let t = traits(p, m);
            (t.temperature, t.reasons, t.effort)
        };
        let older = (true, false, None);
        for m in [
            "claude-haiku-4-5",
            "claude-haiku-4-5-20251001",
            "claude-sonnet-4-20250514",
            "claude-opus-4-1-20250805",
            "claude-3-5-sonnet-20241022",
        ] {
            assert_eq!(t("anthropic", m), older, "{m}");
        }
        for m in [
            "claude-sonnet-5-5",
            "claude-opus-5-5",
            "claude-fable-5-1",
            "claude-opus-4-7",
            "claude-next",
        ] {
            assert_eq!(t("anthropic", m), (false, true, None), "{m}");
        }
        for m in ["gpt-4.1-mini", "gpt-4o", "gpt-4.1"] {
            assert_eq!(t("openai", m), older, "{m}");
        }
        for m in ["gpt-6-luna", "gpt-6.1-sol", "gpt-6-astra", "gpt-5.4", "o3"] {
            assert_eq!(t("openai", m), (false, true, Some("low")), "{m}");
        }
        assert_eq!(t("gemini", "gemini-2.5-flash"), older);
        for m in [
            "gemini-3.8-flash",
            "gemini-3.1-flash-lite",
            "models/gemini-3.5-flash-lite",
        ] {
            assert_eq!(t("gemini", m), (false, true, Some("low")), "{m}");
        }
        assert_eq!(t("openai-compat", "gpt-6.1-sol"), older);
    }

    #[test]
    fn presets_and_names() {
        for (name, want) in [
            ("openai", "openai"),
            ("Anthropic", "anthropic"),
            ("gemini", "gemini"),
        ] {
            assert_eq!(CloudProvider::preset(name, "m").unwrap().name(), want);
        }
        assert!(CloudProvider::preset("mistral", "m").is_err());
        assert!(CloudProvider::preset("openai", "  ").is_err());
        let c = CloudProvider::openai_compat("https://llm.internal.example.com/v1", "m").unwrap();
        assert_eq!(c.name(), "openai-compat");
        assert!(CloudProvider::openai_compat("http://llm.example.com/v1", "m").is_err());
    }

    #[test]
    fn prepared_bytes_are_deterministic_and_keyless() {
        for name in ["openai", "anthropic", "gemini"] {
            let p = CloudProvider::preset(name, "some-model").unwrap();
            let a = p.prepare(&request()).unwrap();
            let b = p.prepare(&request()).unwrap();
            assert_eq!(a, b, "{name}");
            let text = String::from_utf8(a.body.clone()).unwrap();
            assert!(!text.contains(KEY) && !a.url.contains(KEY));
            assert!(text.contains("<<EMAIL_1>>"));
            assert!(a.url.starts_with("https://"));
        }
    }

    #[test]
    fn prepared_host() {
        let host = |name| {
            CloudProvider::preset(name, "m")
                .unwrap()
                .prepare(&request())
                .unwrap()
                .host()
        };
        assert_eq!(host("openai"), "api.openai.com");
        assert_eq!(host("anthropic"), "api.anthropic.com");
        assert_eq!(host("gemini"), "generativelanguage.googleapis.com");
        let p = Prepared {
            url: "https://[2001:db8::1]:8443/x?y=1".into(),
            body: vec![],
        };
        assert_eq!(p.host(), "2001:db8::1");
        let p = Prepared {
            url: "https://Host.Example:8443/x".into(),
            body: vec![],
        };
        assert_eq!(p.host(), "host.example");
    }

    #[test]
    fn headers_carry_the_key_and_never_print_it() {
        let key = Secret::new(KEY);
        let find = |h: &Headers, name: &str| {
            h.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.expose().to_owned())
        };
        let o = CloudProvider::preset("openai", "m").unwrap().headers(&key);
        assert_eq!(find(&o, "authorization").unwrap(), format!("Bearer {KEY}"));
        let g = CloudProvider::preset("gemini", "m").unwrap().headers(&key);
        assert_eq!(find(&g, "authorization").unwrap(), format!("Bearer {KEY}"));
        assert!(find(&g, "x-goog-api-key").is_none());
        let a = CloudProvider::preset("anthropic", "m")
            .unwrap()
            .headers(&key);
        assert_eq!(find(&a, "x-api-key").unwrap(), KEY);
        assert_eq!(find(&a, "anthropic-version").unwrap(), "2023-06-01");
        for h in [&o, &g, &a] {
            assert!(!format!("{h:?}").contains(KEY));
        }
        assert!(!format!("{key:?} {key}").contains(KEY));
    }

    #[test]
    fn error_answers_become_short_provider_errors() {
        let p = CloudProvider::preset("openai", "m").unwrap();
        let body = br#"{"error":{"message":"Incorrect API key provided: sk-abcdefgh***wxyz. Find it at https://platform.openai.com","type":"invalid_request_error"}}"#;
        let LlmError::Provider { status, message } = p.parse(401, body).unwrap_err() else {
            panic!()
        };
        assert_eq!(status, 401);
        assert!(!message.contains("sk-abcdefgh"), "{message}");
        assert!(message.contains("Incorrect API key"));
        // Non-JSON and oversized bodies are cut.
        let big = "x".repeat(5000);
        let LlmError::Provider { message, .. } = p.parse(502, big.as_bytes()).unwrap_err() else {
            panic!()
        };
        assert!(message.chars().count() <= MAX_ERROR_CHARS + 3);
        // Gemini array-wrapped error.
        let g = CloudProvider::preset("gemini", "m").unwrap();
        let LlmError::Provider { message, .. } = g
            .parse(
                400,
                br#"[{"error":{"code":400,"message":"API key not valid."}}]"#,
            )
            .unwrap_err()
        else {
            panic!()
        };
        assert_eq!(message, "API key not valid.");
    }

    #[test]
    fn a_grant_for_other_bytes_is_denied_without_a_connection() {
        let p = CloudProvider::preset("openai", "m").unwrap();
        let prepared = p.prepare(&request()).unwrap();
        let gate = MeetingGate {
            cloud_locked: false,
            sensitive: false,
        };
        let mut grant = CloudGrant::mint(
            NetPolicy::Default,
            &prepared.url,
            b"not these bytes",
            &[gate],
        )
        .unwrap();
        let before = ghi_net::connections_opened();
        let err = p
            .send(&prepared, &mut grant, &Secret::new(KEY))
            .unwrap_err();
        assert!(matches!(err, LlmError::Denied(_)), "{err:?}");
        assert!(!err.to_string().contains(KEY));
        assert_eq!(ghi_net::connections_opened(), before);
    }

    #[test]
    fn keys_are_masked_in_provider_text() {
        let e = LlmError::Provider {
            status: 400,
            message: format!("bad key {KEY}"),
        };
        let LlmError::Provider { message, .. } = scrub(e, &Secret::new(KEY)) else {
            panic!()
        };
        assert!(!message.contains(KEY));
        assert_eq!(
            mask_keys("AIzaSyA1234567890abcdefghij and Bearer abc.def"),
            "[redacted] and [redacted]"
        );
    }
}
