// SPDX-License-Identifier: Apache-2.0
//! One structured call to the local model with validation and retries.

use serde_json::Value;

use crate::template::OutLang;
use crate::validate::{Diagnostics, extract_json};
use crate::{Llm, LlmError, Message, Request, Result, prompt};

/// Retries after invalid output (doc: phase 6 step 4).
pub const MAX_RETRIES: u32 = 2;
/// How much of an invalid reply is echoed back in a retry (only the last one
/// is kept, so a retry needs at most [`RETRY_RESERVE`] more prompt tokens).
const ECHO_BYTES: usize = 3000;

/// Bytes / 3: a deliberately high token estimate for EN and VN text, used to
/// decide what fits in the model's context.
pub fn estimate_tokens(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(3)).unwrap_or(u32::MAX)
}

/// Tokens of prompt around the transcript (system prompt, task, schema).
pub const PROMPT_OVERHEAD: u32 = 1500;
/// Extra prompt tokens a retry can add.
pub const RETRY_RESERVE: u32 = 1200;

pub enum Outcome<T> {
    Done(T),
    /// The reply hit `max_tokens`; the caller should ask for less at once.
    Truncated,
}

/// Runs `req`, parses the JSON reply with `parse`, and on invalid output
/// retries with the error fed back (≤ [`MAX_RETRIES`]).
pub fn complete_json<T>(
    llm: &mut dyn Llm,
    req: Request,
    lang: OutLang,
    diag: &mut Diagnostics,
    mut parse: impl FnMut(&Value, &mut Diagnostics) -> std::result::Result<T, String>,
) -> Result<Outcome<T>> {
    let base = req.messages.clone();
    let mut req = req;
    let mut last_error = String::new();
    for attempt in 0..=MAX_RETRIES {
        if attempt > 0 {
            diag.retries += 1;
        }
        let c = llm.complete(&req)?;
        diag.requests += 1;
        diag.tokens_in += u64::from(c.tokens_in);
        diag.tokens_out += u64::from(c.tokens_out);
        if c.truncated {
            return Ok(Outcome::Truncated);
        }
        // Parse into a scratch copy so a failed attempt's counts don't stick.
        let mut scratch = Diagnostics::default();
        match extract_json(&c.text).and_then(|v| parse(&v, &mut scratch)) {
            Ok(value) => {
                diag.add(&scratch);
                return Ok(Outcome::Done(value));
            }
            Err(e) => {
                last_error = e;
                let mut end = c.text.len().min(ECHO_BYTES);
                while !c.text.is_char_boundary(end) {
                    end -= 1;
                }
                req.messages = base.clone();
                req.messages.push(Message::assistant(&c.text[..end]));
                req.messages
                    .push(Message::user(prompt::retry(lang, &last_error)));
            }
        }
    }
    Err(LlmError::InvalidOutput(last_error))
}
