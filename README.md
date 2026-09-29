# Ghira

Ghira is an offline-first AI meeting note taker for English and Vietnamese, with
live speaker diarization. Audio, transcripts and notes stay on your device.
Cloud AI is optional, per meeting, and sends transcript text only, never audio.

> Status: early development (phase 1 of the v1 roadmap). Nothing to install yet.

## Principles

- **Offline by default.** No telemetry. See [PRIVACY.md](PRIVACY.md) for exactly what may leave your device.
- **Open source**, [Apache-2.0](LICENSE). Contributions use a DCO sign-off; see [CONTRIBUTING.md](CONTRIBUTING.md).
- **Your voice is yours.** Voice profiles need explicit consent.

## Repository layout

| Path | What |
|---|---|
| `crates/` | Rust core: capture, speech, storage, LLM client, network policy, CLI |
| `apps/desktop/` | Tauri 2 desktop app (React + TypeScript) |
| `apps/mobile/` | iOS and Android app (later phases) |
| `packages/` | Shared UI components and translations |
| `native/` | Platform audio plugins (Swift, Kotlin) |
| `third_party/NeMo-Speech.cpp` | NVIDIA speech runtime (git submodule) |
| `tools/` | Eval kit and repo scripts |

## Build

See [CONTRIBUTING.md](CONTRIBUTING.md#development-setup).

## Security

Report vulnerabilities privately; see [SECURITY.md](SECURITY.md).

"Ghira" and its logo are not covered by the code license; see [TRADEMARKS.md](TRADEMARKS.md).
