# Ghira: guidance for Claude Code

Offline-first AI meeting note taker (EN + VN, live diarization). Apache-2.0.
Product name **Ghira**; "Ghi" is the codename used in `Plans/`, crate names (`ghi-*`) and the repo folder.

Specs, designs and the v1 roadmap live in `Plans/` (see `Plans/CLAUDE.md`).
`Plans/` is kept **local only** and is not committed to this repo; team
members get it separately.

## Invariants

- Offline by default: no user content leaves the device except explicit opt-ins
  (see `PRIVACY.md`). No telemetry.
- All network I/O goes through `crates/ghi-net`. The webview never does network
  I/O (CSP + ESLint rule).
- Cloud AI is opt-in per meeting and sends transcript text only, never audio.
- Voiceprints need explicit consent. Speakers are shown as color + initial,
  never color alone.
- No GPL/AGPL/SSPL/non-commercial dependencies in shipped binaries; no bundled
  FFmpeg. Models and eval recordings never enter git.

## Conventions

- `// SPDX-License-Identifier: Apache-2.0` (or `#`) at the top of every source file.
- Rust `snake_case`; TS/JS files `kebab-case`.
- Conventional Commits, signed off (`git commit -s`).
- Tauri commands via tauri-specta; regenerate bindings with
  `GHI_UPDATE_BINDINGS=1 cargo test -p ghi-desktop`.

## Checks

```sh
pnpm build && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
pnpm lint && pnpm typecheck && pnpm test && pnpm --filter @ghi/desktop test:e2e
./tools/scripts/check-spdx.sh && ./tools/scripts/check-net-egress.sh
cargo deny check licenses bans advisories sources
# speech engines (phase 3; needs tools/scripts/build-nemo.sh and fetch-models.sh first):
cargo clippy -p ghi-speech -p ghi-cli --all-targets --features ghi-cli/nemo -- -D warnings
cargo test -p ghi-speech -p ghi-cli --features ghi-cli/nemo
# eval kit (phase 2), from tools/eval:
uv sync --locked && uv run ruff check && uv run ruff format --check && uv run pytest -q
```

Audio capture: `crates/ghi-audio` (rings, 16 kHz resample, AEC via `sonora`,
Ogg Opus, meeting detect) + `native/macos/GhiAudioMac` (Swift, C ABI in its
`include/ghi_audio_mac.h`, built by `ghi-audio/build.rs`); `ghi record|recover|detect`.

Storage: `crates/ghi-store` (SQLCipher, per-meeting DEKs, encrypted audio
bundles, VN-folded FTS5 search, crypto-shred delete, key stores) and
`ghi store ...`. Debug-only file key store; `tools/scripts/check-no-dev-key.sh`
guards release binaries.

Notes engine: `crates/ghi-llm` (templates in `templates/*.toml`, generated
JSON schemas, map-reduce notes with citations, enhance, Ask, redaction, send
preview, cloud providers) + `crates/ghi-llm-worker` (llama.cpp over stdio, a
separate process); `ghi notes|ask|keys` and `ghi store notes`. Local model:
`tools/scripts/fetch-models.sh qwen3-4b`, then `cargo build -p ghi-llm-worker`
(the golden tests skip without both). Cloud sends only go through `ghi-net`'s
`CloudGrant`, bound to the exact previewed bytes.

Speech engines: `crates/ghi-speech` (NeMo-Speech.cpp FFI, feature `nemo`); models
pinned in `crates/ghi-models/registry.toml`; decision record `Plans/docs/06`.

Eval kit: `tools/eval` (`ghi-eval`, Python 3.11 + uv). The data, `ghi` CLI and
report contract is `tools/eval/docs/formats.md`. Recordings, transcripts and
run directories never enter git; only aggregate reports leave the customer.
