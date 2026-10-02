// SPDX-License-Identifier: Apache-2.0
//! Entry point of the LLM worker. Spawned by `ghi-llm` with `std::process`;
//! it speaks the JSON-lines protocol of the library on stdin and a private
//! copy of the original stdout, and runs llama.cpp in-process. Killing the
//! process is how the model is unloaded.

use std::io::{BufRead, BufReader, Write};
use std::num::NonZeroU32;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::Instant;

use ghi_llm_worker::{Body, Hello, Op, Reply, Request, WireMessage};
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::{KvCacheType, LlamaContextParams, LlamaPoolingType};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use llama_cpp_2::token::data::LlamaTokenData;
use llama_cpp_2::token::data_array::LlamaTokenDataArray;

/// Prompt tokens decoded per batch.
const N_BATCH: usize = 512;
/// A progress line every this many output tokens.
const PROGRESS_EVERY: u32 = 16;
/// `LLAMA_FLASH_ATTN_TYPE_ENABLED`; required for a quantized V cache.
const FLASH_ATTN_ENABLED: i32 = 1;

/// Prompt template families the worker can build by hand.
#[derive(Clone, Copy)]
enum ChatFormat {
    Qwen3,
}

impl ChatFormat {
    fn parse(name: &str) -> Result<ChatFormat, String> {
        match name {
            "qwen3" => Ok(ChatFormat::Qwen3),
            other => Err(format!("unknown chat_format {other:?}")),
        }
    }

    /// The prompt as text pieces; only pieces marked `special` may be parsed
    /// into control tokens.
    fn segments(self, messages: &[WireMessage]) -> Vec<Segment> {
        match self {
            ChatFormat::Qwen3 => chatml_segments(messages),
        }
    }
}

/// A piece of prompt text. Scaffolding is `special`; message content is not.
struct Segment {
    text: String,
    special: bool,
}

struct Engine {
    model: LlamaModel,
    n_ctx: u32,
    seed: u32,
    format: ChatFormat,
}

struct Embedder {
    model: LlamaModel,
    max_tokens: usize,
    dim: usize,
}

struct Worker {
    out: Box<dyn Write>,
    backend: LlamaBackend,
    engine: Option<Engine>,
    embedder: Option<Embedder>,
}

fn main() {
    if let Some(dir) = ghi_diag::dir_from_env() {
        ghi_diag::install_panic_hook("worker", dir, || "worker".into());
    }
    let mut out = take_protocol_stdout();
    // Hello first: backend init (Metal) can be slow, and the parent only needs
    // to know the process is alive and speaks its protocol.
    let hello = serde_json::to_string(&Hello::current()).expect("hello serializes");
    if writeln!(out, "{hello}").and_then(|_| out.flush()).is_err() {
        std::process::exit(1);
    }
    let debug = std::env::var_os("GHI_LLM_DEBUG").is_some_and(|v| v == "1");
    let mut backend = match LlamaBackend::init() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("ghi-llm-worker: backend init failed: {e}");
            std::process::exit(1);
        }
    };
    if !debug {
        backend.void_logs();
    }
    let mut w = Worker {
        out,
        backend,
        engine: None,
        embedder: None,
    };
    let stdin = std::io::stdin();
    // Ends at EOF: the parent closing our stdin is a clean exit.
    for line in BufReader::new(stdin.lock()).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(_) => {
                w.send(
                    0,
                    Body::Error {
                        message: "malformed request".into(),
                    },
                );
                continue;
            }
        };
        let id = req.id;
        if matches!(req.op, Op::Shutdown) {
            break;
        }
        let body = match catch_unwind(AssertUnwindSafe(|| w.handle(id, req.op))) {
            Ok(Ok(b)) => b,
            Ok(Err(message)) => Body::Error { message },
            Err(_) => Body::Error {
                message: "worker panicked".into(),
            },
        };
        w.send(id, body);
    }
}

/// Keep the real stdout for protocol replies and point fd 1 at stderr, so a C
/// `printf` from llama.cpp or ggml can never corrupt the protocol.
#[cfg(unix)]
fn take_protocol_stdout() -> Box<dyn Write> {
    use std::fs::File;
    use std::os::fd::FromRawFd;
    // SAFETY: plain fd juggling at process start, before any other thread runs;
    // the duplicated fd is owned by the returned File.
    unsafe {
        let fd = libc::dup(1);
        assert!(fd >= 0, "dup(stdout) failed");
        assert!(libc::dup2(2, 1) >= 0, "dup2(stderr, stdout) failed");
        Box::new(File::from_raw_fd(fd))
    }
}

/// Elsewhere there is no fd redirection: the protocol goes to plain stdout and
/// only the voided llama logs (see `main`) protect it from library output.
#[cfg(not(unix))]
fn take_protocol_stdout() -> Box<dyn Write> {
    Box::new(std::io::stdout())
}

impl Worker {
    fn send(&mut self, id: u64, body: Body) {
        let line = serde_json::to_string(&Reply { id, body }).expect("reply serializes");
        self.send_line(&line);
    }

    fn send_line(&mut self, line: &str) {
        // The parent is gone if this fails; nothing more to do.
        if writeln!(self.out, "{line}")
            .and_then(|_| self.out.flush())
            .is_err()
        {
            std::process::exit(1);
        }
    }

    fn handle(&mut self, id: u64, op: Op) -> Result<Body, String> {
        match op {
            Op::Load {
                model_path,
                n_ctx,
                n_gpu_layers,
                seed,
                chat_format,
            } => self.load(&model_path, n_ctx, n_gpu_layers, seed, &chat_format),
            Op::Complete {
                messages,
                schema,
                max_tokens,
                temperature,
            } => {
                let engine = self.engine.as_ref().ok_or("no model loaded")?;
                let out = &mut self.out;
                let mut progress = |tokens_in_done: u32, tokens_out: u32| {
                    let line = serde_json::to_string(&Reply {
                        id,
                        body: Body::Progress {
                            tokens_in_done,
                            tokens_out,
                        },
                    })
                    .expect("reply serializes");
                    if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                        std::process::exit(1);
                    }
                };
                engine.complete(
                    &self.backend,
                    &messages,
                    schema.as_ref(),
                    max_tokens,
                    temperature,
                    &mut progress,
                )
            }
            Op::Count { text } => {
                let engine = self.engine.as_ref().ok_or("no model loaded")?;
                let plain = [Segment {
                    text,
                    special: false,
                }];
                let tokens = engine.tokenize(&plain)?.len();
                Ok(Body::Counted {
                    tokens: u32::try_from(tokens).unwrap_or(u32::MAX),
                })
            }
            Op::LoadEmbed {
                model_path,
                max_tokens,
                n_gpu_layers,
            } => self.load_embed(&model_path, max_tokens, n_gpu_layers),
            Op::Embed { texts } => {
                let e = self.embedder.as_ref().ok_or("no embedding model loaded")?;
                let vectors = e.embed(&self.backend, &texts)?;
                Ok(Body::Embedded {
                    dim: e.dim as u32,
                    vectors,
                })
            }
            Op::Health => Ok(Body::Health {
                loaded: self.engine.is_some() || self.embedder.is_some(),
            }),
            Op::Shutdown => unreachable!("handled by the read loop"),
        }
    }

    fn load(
        &mut self,
        path: &str,
        n_ctx: u32,
        n_gpu_layers: u32,
        seed: u32,
        chat_format: &str,
    ) -> Result<Body, String> {
        let t = Instant::now();
        let format = ChatFormat::parse(chat_format)?;
        if !Path::new(path).is_file() {
            return Err("model file not found".into());
        }
        if n_ctx == 0 {
            return Err("n_ctx must be > 0".into());
        }
        // Free a previous model before loading the next one.
        self.engine = None;
        self.embedder = None;
        let params = LlamaModelParams::default().with_n_gpu_layers(n_gpu_layers);
        let model = LlamaModel::load_from_file(&self.backend, path, &params)
            .map_err(|e| format!("model load failed: {e}"))?;
        self.engine = Some(Engine {
            model,
            n_ctx,
            seed,
            format,
        });
        Ok(Body::Loaded {
            n_ctx,
            load_s: t.elapsed().as_secs_f64(),
        })
    }

    fn load_embed(
        &mut self,
        path: &str,
        max_tokens: u32,
        n_gpu_layers: u32,
    ) -> Result<Body, String> {
        let t = Instant::now();
        if !Path::new(path).is_file() {
            return Err("model file not found".into());
        }
        if max_tokens == 0 {
            return Err("max_tokens must be > 0".into());
        }
        self.engine = None;
        self.embedder = None;
        let params = LlamaModelParams::default().with_n_gpu_layers(n_gpu_layers);
        let model = LlamaModel::load_from_file(&self.backend, path, &params)
            .map_err(|e| format!("model load failed: {e}"))?;
        let dim = usize::try_from(model.n_embd_out()).map_err(|_| "bad embedding size")?;
        // One token is the closing EOS the pooled vector is read from.
        let max_tokens = (max_tokens as usize).min(model.n_ctx_train() as usize);
        self.embedder = Some(Embedder {
            model,
            max_tokens,
            dim,
        });
        Ok(Body::EmbedLoaded {
            dim: dim as u32,
            load_s: t.elapsed().as_secs_f64(),
        })
    }
}

impl Embedder {
    /// Qwen3-Embedding reads the vector of the last token, which must be the
    /// end-of-text token: it is appended here (the GGUF tokenizer does not).
    /// Each text is its own sequence in a fresh context.
    fn embed(&self, backend: &LlamaBackend, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let n_ctx = self.max_tokens + 1;
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx as u32))
            .with_n_batch(n_ctx as u32)
            .with_n_ubatch(n_ctx as u32)
            .with_embeddings(true)
            .with_pooling_type(LlamaPoolingType::Last);
        let mut ctx = self
            .model
            .new_context(backend, ctx_params)
            .map_err(|e| format!("context creation failed: {e}"))?;
        let mut batch = LlamaBatch::new(n_ctx, 1);
        let eos = self.model.token_eos();
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            let plain = [Segment {
                text: text.clone(),
                special: false,
            }];
            let mut tokens = tokenize_segments(&self.model, &plain)?;
            tokens.truncate(self.max_tokens);
            tokens.push(eos);
            batch.clear();
            batch
                .add_sequence(&tokens, 0, true)
                .map_err(|e| format!("batch: {e}"))?;
            ctx.clear_kv_cache();
            ctx.decode(&mut batch)
                .map_err(|e| format!("decode failed: {e}"))?;
            let raw = ctx
                .embeddings_seq_ith(0)
                .map_err(|e| format!("embedding failed: {e}"))?;
            out.push(l2_normalized(raw));
        }
        Ok(out)
    }
}

/// `v` scaled to unit length (a zero vector stays zero).
fn l2_normalized(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter().map(|x| x / norm).collect()
    } else {
        v.to_vec()
    }
}

impl Engine {
    fn complete(
        &self,
        backend: &LlamaBackend,
        messages: &[WireMessage],
        schema: Option<&serde_json::Value>,
        max_tokens: u32,
        temperature: f32,
        progress: &mut dyn FnMut(u32, u32),
    ) -> Result<Body, String> {
        let t = Instant::now();
        let prompt_tokens = self.tokenize(&self.format.segments(messages))?;
        let tokens_in = prompt_tokens.len();
        if tokens_in as u64 + u64::from(max_tokens) > u64::from(self.n_ctx) {
            return Err(format!(
                "prompt ({tokens_in} tokens) + max_tokens ({max_tokens}) exceeds the context ({})",
                self.n_ctx
            ));
        }

        let mut sampler = self.sampler(schema, temperature)?;
        // A fresh context per request: no KV state carries over between notes.
        // q8_0 K/V halves the cache; a quantized V cache needs flash attention.
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(self.n_ctx))
            .with_n_batch(N_BATCH as u32)
            .with_type_k(KvCacheType::Q8_0)
            .with_type_v(KvCacheType::Q8_0)
            .with_flash_attention_policy(FLASH_ATTN_ENABLED);
        let mut ctx = self
            .model
            .new_context(backend, ctx_params)
            .map_err(|e| format!("context creation failed: {e}"))?;
        let mut batch = LlamaBatch::new(N_BATCH, 1);

        // Prompt, in chunks; only the last token needs logits.
        let mut pos = 0i32;
        let chunks: Vec<_> = prompt_tokens.chunks(N_BATCH).collect();
        for (ci, chunk) in chunks.iter().enumerate() {
            batch.clear();
            for (i, tok) in chunk.iter().enumerate() {
                let last = ci + 1 == chunks.len() && i + 1 == chunk.len();
                batch
                    .add(*tok, pos, &[0], last)
                    .map_err(|e| format!("batch: {e}"))?;
                pos += 1;
            }
            ctx.decode(&mut batch)
                .map_err(|e| format!("decode failed: {e}"))?;
            progress(pos as u32, 0);
        }

        // Token pieces are raw bytes and a character may span tokens, so the
        // bytes are joined and decoded once at the end.
        let mut bytes: Vec<u8> = Vec::new();
        let mut tokens_out = 0u32;
        let mut truncated = true;
        while tokens_out < max_tokens {
            let tok = sampler.pick(&ctx, batch.n_tokens() - 1);
            if self.model.is_eog_token(tok) {
                truncated = false;
                break;
            }
            sampler.accept(tok);
            let piece = self
                .model
                .token_to_piece_bytes(tok, 32, false, None)
                .map_err(|e| format!("detokenize failed: {e}"))?;
            bytes.extend_from_slice(&piece);
            tokens_out += 1;
            if tokens_out.is_multiple_of(PROGRESS_EVERY) {
                progress(tokens_in as u32, tokens_out);
            }
            batch.clear();
            batch
                .add(tok, pos, &[0], true)
                .map_err(|e| format!("batch: {e}"))?;
            pos += 1;
            ctx.decode(&mut batch)
                .map_err(|e| format!("decode failed: {e}"))?;
        }

        let text = strip_think(&String::from_utf8_lossy(&bytes));
        Ok(Body::Completed {
            text,
            tokens_in: tokens_in as u32,
            tokens_out,
            truncated,
            wall_s: t.elapsed().as_secs_f64(),
        })
    }

    fn tokenize(&self, segments: &[Segment]) -> Result<Vec<LlamaToken>, String> {
        tokenize_segments(&self.model, segments)
    }

    /// The Qwen3 non-thinking sampling settings (never greedy, which loops on
    /// repeated text), plus the grammar of the schema when one is given.
    fn sampler(
        &self,
        schema: Option<&serde_json::Value>,
        temperature: f32,
    ) -> Result<Sampling, String> {
        let grammar = match schema {
            Some(schema) => {
                let gbnf = llama_cpp_2::json_schema_to_grammar(&schema.to_string())
                    .map_err(|e| format!("schema to grammar failed: {e}"))?;
                Some(
                    LlamaSampler::grammar(&self.model, &gbnf, "root")
                        .map_err(|e| format!("grammar rejected: {e}"))?,
                )
            }
            None => None,
        };
        let chain = LlamaSampler::chain_simple([
            LlamaSampler::top_k(20),
            LlamaSampler::top_p(0.8, 1),
            LlamaSampler::min_p(0.0, 1),
            LlamaSampler::temp(temperature.max(0.1)),
            LlamaSampler::dist(self.seed),
        ]);
        Ok(Sampling { chain, grammar })
    }
}

/// `str_to_token` always parses special tokens, so untrusted text is fed
/// in pieces that cannot form one: every special token here starts with
/// `<`, which is tokenized on its own.
fn tokenize_segments(model: &LlamaModel, segments: &[Segment]) -> Result<Vec<LlamaToken>, String> {
    let mut out = Vec::new();
    let mut add = |text: &str| -> Result<(), String> {
        if !text.is_empty() {
            out.extend(
                model
                    .str_to_token(text, AddBos::Never)
                    .map_err(|e| format!("tokenize failed: {e}"))?,
            );
        }
        Ok(())
    };
    for seg in segments {
        if seg.special {
            add(&seg.text)?;
            continue;
        }
        let mut rest = seg.text.as_str();
        while let Some(i) = rest.find('<') {
            add(&rest[..i])?;
            add("<")?;
            rest = &rest[i + 1..];
        }
        add(rest)?;
    }
    Ok(out)
}

/// The sampler chain and, separately, the grammar. Running the grammar over
/// the whole vocabulary every token is slow, so (as llama.cpp's
/// `common_sampler` does) a token is first drawn from the unconstrained chain
/// and only checked against the grammar; the full grammar-first path runs
/// only when that token is rejected.
struct Sampling {
    chain: LlamaSampler,
    grammar: Option<LlamaSampler>,
}

impl Sampling {
    /// Choose the next token from the logits at batch index `idx`. Does not
    /// accept it; call [`Sampling::accept`] once the token is kept.
    fn pick(&mut self, ctx: &LlamaContext, idx: i32) -> LlamaToken {
        let mut cand = ctx.token_data_array_ith(idx);
        cand.apply_sampler(&self.chain);
        let tok = cand.selected_token().expect("dist selects a token");
        let Some(grammar) = &self.grammar else {
            return tok;
        };
        // Fast path: only this token's logit goes through the grammar.
        let logit = ctx.get_logits_ith(idx)[tok.0 as usize];
        let mut one = LlamaTokenDataArray::new(vec![LlamaTokenData::new(tok, logit, 0.0)], false);
        one.apply_sampler(grammar);
        if one.data[0].logit() != f32::NEG_INFINITY {
            return tok;
        }
        // Rejected: grammar over all candidates, then the chain, then draw again.
        let mut full = ctx.token_data_array_ith(idx);
        full.apply_sampler(grammar);
        full.apply_sampler(&self.chain);
        full.selected_token().expect("dist selects a token")
    }

    /// Accept the kept token, exactly once per sampler (grammar state advances).
    /// The samplers' own `sample` would accept internally, which `pick` avoids.
    fn accept(&mut self, tok: LlamaToken) {
        if let Some(g) = &mut self.grammar {
            g.accept(tok);
        }
        self.chain.accept(tok);
    }
}

/// Qwen3 ChatML in no-think mode: the assistant turn opens with an empty
/// think block so the model answers directly.
fn chatml_segments(messages: &[WireMessage]) -> Vec<Segment> {
    let mut v = Vec::new();
    for m in messages {
        v.push(Segment {
            text: format!("<|im_start|>{}\n", m.role),
            special: true,
        });
        v.push(Segment {
            text: m.content.clone(),
            special: false,
        });
        v.push(Segment {
            text: "<|im_end|>\n".into(),
            special: true,
        });
    }
    v.push(Segment {
        text: "<|im_start|>assistant\n<think>\n\n</think>\n\n".into(),
        special: true,
    });
    v
}

/// Drop a leading `<think>...</think>` block, if the model emitted one anyway.
fn strip_think(text: &str) -> String {
    let t = text.trim_start();
    if let Some(rest) = t.strip_prefix("<think>")
        && let Some(end) = rest.find("</think>")
    {
        return rest[end + "</think>".len()..].trim_start().to_string();
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatml_ends_with_empty_think() {
        let s: String = chatml_segments(&[
            WireMessage {
                role: "system".into(),
                content: "s".into(),
            },
            WireMessage {
                role: "user".into(),
                content: "u".into(),
            },
        ])
        .into_iter()
        .map(|seg| seg.text)
        .collect();
        assert_eq!(
            s,
            "<|im_start|>system\ns<|im_end|>\n<|im_start|>user\nu<|im_end|>\n\
             <|im_start|>assistant\n<think>\n\n</think>\n\n"
        );
    }

    #[test]
    fn normalizing_gives_unit_length() {
        let v = l2_normalized(&[3.0, 4.0]);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
        assert_eq!(l2_normalized(&[0.0, 0.0]), vec![0.0, 0.0]);
    }

    #[test]
    fn think_block_is_stripped() {
        assert_eq!(strip_think("<think>x</think>\n\n{\"a\":1}"), "{\"a\":1}");
        assert_eq!(strip_think("{\"a\":1}"), "{\"a\":1}");
    }
}
