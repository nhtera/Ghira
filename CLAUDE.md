# Ghi: guidance for Claude Code

Offline-first AI meeting note taker (EN + VN, live diarization). Apache-2.0.

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
```
