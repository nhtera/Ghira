// SPDX-License-Identifier: Apache-2.0
//! Cloud AI from the command line (doc 02 §K): opt-in per run, transcript
//! text only, redacted, and never sent before the exact payload was shown.
//!
//! `--preview` prints the request byte for byte (`ghi.send-preview/1`) with
//! its SHA-256 and sends nothing. `--confirm-send <sha256>` sends it, and
//! only if the payload built now is the same bytes. The send itself goes
//! through `ghi-net` under a `CloudGrant` bound to those bytes; a stored
//! meeting that is cloud-locked or sensitive refuses it. If the provider
//! fails, the local model writes the notes instead.
//!
//! API keys live in the OS keystore (`ghi keys set <provider>`, key read
//! from stdin); never in arguments, files or output.

use std::path::PathBuf;

use ghi_llm::Request;
use ghi_llm::cloud::{CloudProvider, Prepared};
use ghi_llm::preview::{Prices, SendPreview, preview};
use ghi_net::{CloudGrant, MeetingGate, NetPolicy, Secret};
use ghi_store::keys::secrets::SecretStore;
use serde_json::{Value, json};

use crate::contract::{ErrorCode, ErrorDoc};

#[derive(Debug, Clone, clap::Args)]
pub struct CloudArgs {
    /// `local` (default, offline) or a cloud provider: openai, anthropic, gemini.
    #[arg(long, default_value = "local")]
    pub provider: String,
    /// The cloud model (required with a cloud provider).
    #[arg(long)]
    pub cloud_model: Option<String>,
    /// Base URL of an OpenAI-compatible server (with --provider openai); its
    /// key is stored under the host (`ghi keys set <host>`).
    #[arg(long)]
    pub base_url: Option<String>,
    /// Print the exact request (`ghi.send-preview/1`) and send nothing.
    #[arg(long)]
    pub preview: bool,
    /// Send, if the payload's SHA-256 is this value from --preview.
    #[arg(long, value_name = "SHA256")]
    pub confirm_send: Option<String>,
    /// A price table (TOML) instead of the built-in one.
    #[arg(long)]
    pub prices: Option<PathBuf>,
    /// Block all internet traffic (a cloud provider is refused).
    #[arg(long)]
    pub strict_offline: bool,
    /// More names to redact (comma-separated), besides the speakers' names.
    #[arg(long, value_delimiter = ',')]
    pub redact_names: Vec<String>,
}

impl CloudArgs {
    /// The cloud provider, or `None` for the local model.
    pub fn provider(&self) -> Result<Option<CloudProvider>, ErrorDoc> {
        if self.provider == "local" {
            if self.preview || self.confirm_send.is_some() {
                return Err(bad("--preview and --confirm-send need a cloud --provider"));
            }
            return Ok(None);
        }
        let model = self
            .cloud_model
            .as_deref()
            .ok_or_else(|| bad("a cloud provider needs --cloud-model"))?;
        let p = match (&self.base_url, self.provider.as_str()) {
            (Some(url), "openai") => CloudProvider::openai_compat(url, model),
            (Some(_), _) => return Err(bad("--base-url is for --provider openai")),
            (None, name) => CloudProvider::preset(name, model),
        };
        p.map(Some).map_err(crate::cmd::notes::llm_error)
    }
}

fn bad(msg: &str) -> ErrorDoc {
    ErrorDoc::new(ErrorCode::BadInput, msg)
}

/// What a cloud step produced.
pub enum Exchange {
    /// `--preview`: the preview was printed; nothing was sent.
    Previewed,
    /// The provider's reply text, and a description of the send.
    Reply {
        completion: ghi_llm::Completion,
        sent: Value,
    },
    /// Sending failed (network, provider error): fall back to local. `sent`
    /// is set when the request may have left the device (log it).
    Failed { reason: String, sent: Option<Value> },
}

/// Previews or sends `req`. Refusals (not confirmed, no key, a locked or
/// sensitive meeting, strict offline) are errors; provider failures are
/// [`Exchange::Failed`] so the caller can fall back to the local model.
pub fn exchange(
    args: &CloudArgs,
    provider: &CloudProvider,
    req: &Request,
    gates: &[MeetingGate],
    redactions: &[(&'static str, usize)],
) -> Result<Exchange, ErrorDoc> {
    // A cloud-locked or sensitive meeting is refused before anything is
    // built or shown (the grant would refuse the send anyway).
    if gates.iter().any(|g| g.cloud_locked || g.sensitive) {
        return Err(bad(
            "cloud AI is off for this meeting (cloud-locked or sensitive)",
        ));
    }
    let prepared: Prepared = provider
        .prepare(req)
        .map_err(crate::cmd::notes::llm_error)?;
    let prices = match &args.prices {
        Some(p) => Prices::from_file(p).map_err(crate::cmd::notes::llm_error)?,
        None => Prices::builtin(),
    };
    let pv: SendPreview = preview(provider, &prepared, &prices);
    if args.preview {
        let mut doc = serde_json::to_value(&pv)
            .map_err(|e| ErrorDoc::new(ErrorCode::Internal, e.to_string()))?;
        doc["schema"] = json!("ghi.send-preview/1");
        doc["redactions"] = redactions
            .iter()
            .map(|(kind, n)| json!({"kind": kind, "count": n}))
            .collect();
        crate::emit(&doc)?;
        return Ok(Exchange::Previewed);
    }
    match &args.confirm_send {
        None => {
            return Err(bad(
                "nothing is sent without a confirmed preview: run with --preview, \
                 then pass its sha256 to --confirm-send",
            ));
        }
        Some(sha) if !sha.eq_ignore_ascii_case(&pv.sha256) => {
            return Err(bad(
                "the payload changed since the preview (--confirm-send does not match); preview again",
            ));
        }
        Some(_) => {}
    }
    let policy = if args.strict_offline {
        NetPolicy::StrictOffline
    } else {
        NetPolicy::Default
    };
    let mut grant = CloudGrant::mint(policy, &prepared.url, &prepared.body, gates)
        .map_err(|e| ErrorDoc::new(ErrorCode::BadInput, format!("cloud send refused: {e}")))?;
    let key = api_key(&key_account(provider, &prepared))?;
    let sent = |tokens_in: u32, tokens_out: u32| {
        json!({
            "provider": provider.name(),
            "model": provider.model,
            "host": pv.host,
            "sha256": pv.sha256,
            "tokens_in": tokens_in,
            "tokens_out": tokens_out,
        })
    };
    match provider.send(&prepared, &mut grant, &key) {
        Ok(completion) => Ok(Exchange::Reply {
            sent: sent(completion.tokens_in, completion.tokens_out),
            completion,
        }),
        Err(e) => Ok(Exchange::Failed {
            reason: e.to_string(),
            // Once a send was attempted, the body may have left the device.
            sent: Some(sent(0, 0)),
        }),
    }
}

/// Keystore account of a provider's API key.
fn account(provider: &str) -> String {
    format!("provider-{provider}")
}

/// A preset's name, or for another OpenAI-compatible server its host (one key
/// per server).
fn key_account(provider: &CloudProvider, prepared: &Prepared) -> String {
    match provider.name() {
        "openai-compat" => prepared.host(),
        name => name.to_string(),
    }
}

fn api_key(name: &str) -> Result<Secret, ErrorDoc> {
    let bytes = secrets()?
        .get(&account(name))
        .map_err(crate::keystore::store_error)?
        .ok_or_else(|| {
            bad(&format!(
                "no API key for {name}: run `ghi keys set {name}` (key on stdin)"
            ))
        })?;
    let key = std::str::from_utf8(&bytes).map_err(|_| bad("the stored API key is not text"))?;
    Ok(Secret::new(key))
}

/// Where provider keys live: the OS keystore; debug builds default to files
/// under `$GHI_DEV_SECRETS_DIR` (or `~/.ghira-dev/secrets`) unless
/// `GHI_KEYSTORE=keychain`, like the master key.
fn secrets() -> Result<Box<dyn SecretStore>, ErrorDoc> {
    #[cfg(debug_assertions)]
    if std::env::var("GHI_KEYSTORE").as_deref() != Ok("keychain") {
        let dir = std::env::var_os("GHI_DEV_SECRETS_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".ghira-dev/secrets"))
            })
            .ok_or_else(|| bad("set GHI_DEV_SECRETS_DIR"))?;
        return Ok(Box::new(ghi_store::keys::secrets::FileSecrets::new(dir)));
    }
    platform_secrets()
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn platform_secrets() -> Result<Box<dyn SecretStore>, ErrorDoc> {
    Ok(Box::new(ghi_store::keys::secrets::KeychainSecrets::new(
        crate::keystore::KEYCHAIN_SERVICE,
    )))
}

#[cfg(windows)]
fn platform_secrets() -> Result<Box<dyn SecretStore>, ErrorDoc> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or_else(|| bad("LOCALAPPDATA is not set"))?;
    Ok(Box::new(ghi_store::keys::secrets::DpapiSecrets::new(
        PathBuf::from(base)
            .join("Ghira")
            .join("cli")
            .join("secrets"),
    )))
}

#[cfg(not(any(target_os = "macos", target_os = "ios", windows)))]
fn platform_secrets() -> Result<Box<dyn SecretStore>, ErrorDoc> {
    Err(ErrorDoc::new(
        ErrorCode::NotImplemented,
        "keys: no OS key store on this platform yet",
    ))
}

const PROVIDERS: &[&str] = &["openai", "anthropic", "gemini"];

/// A preset provider, or the host of an OpenAI-compatible server
/// (`llm.example.com`, as in its --base-url).
fn check_provider(provider: &str) -> Result<(), ErrorDoc> {
    let host = provider.contains('.')
        && provider.len() <= 55
        && provider
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'));
    if PROVIDERS.contains(&provider) || host {
        Ok(())
    } else {
        Err(bad(&format!(
            "unknown provider `{provider}` (openai, anthropic, gemini, or a server's host)"
        )))
    }
}

/// `ghi keys set <provider>`: stores the key read from stdin's first line.
pub fn keys_set(provider: &str) -> Result<(), ErrorDoc> {
    check_provider(provider)?;
    let key = crate::cmd::store::read_secret_line("API key")?;
    secrets()?
        .set(&account(provider), key.as_bytes())
        .map_err(crate::keystore::store_error)?;
    crate::emit(&json!({"schema": "ghi.keys/1", "provider": provider, "stored": true}))
}

/// `ghi keys delete <provider>`.
pub fn keys_delete(provider: &str) -> Result<(), ErrorDoc> {
    check_provider(provider)?;
    secrets()?
        .delete(&account(provider))
        .map_err(crate::keystore::store_error)?;
    crate::emit(&json!({"schema": "ghi.keys/1", "provider": provider, "stored": false}))
}

/// `ghi keys status`: which providers have a key (never the key).
pub fn keys_status() -> Result<(), ErrorDoc> {
    let store = secrets()?;
    let mut out = Vec::new();
    for p in PROVIDERS {
        let has = store
            .get(&account(p))
            .map_err(crate::keystore::store_error)?
            .is_some();
        out.push(json!({"provider": p, "stored": has}));
    }
    crate::emit(&json!({"schema": "ghi.keys/1", "providers": out}))
}
