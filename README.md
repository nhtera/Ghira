# Ghira

Ghira is an offline-first AI meeting note taker for English and Vietnamese, with
live speaker diarization. Audio, transcripts and notes stay on your device.
Cloud AI is optional, per meeting, and sends transcript text only, never audio.

> Status: **macOS alpha in preparation** (Apple Silicon, macOS 14.2+). Windows,
> iOS and Android follow. There is no signed download yet; you can build it.

## What it does

- **Record** a call (your mic + the meeting app's audio) or a room, with a live
  transcript in English, Vietnamese or both, and speakers told apart as they talk.
- **Notes** written on your Mac by a local model: summary, decisions, action
  items with owners, open questions, each sentence linked to the moment it was
  said. Your own notes are kept and expanded.
- **Review**: search every meeting (accents optional), play any cited moment,
  correct the transcript, rename speakers, export to Markdown, Word, text,
  subtitles or Obsidian, draft a follow-up email.
- **Import** recordings (Voice Memos, Zoom, Plaud, any audio or video file).
- **Optional cloud AI**, per meeting: you see the exact text before anything is
  sent, names can be hidden, audio never leaves the device.

## Quick start (from source, macOS)

Needs Rust (rustup), Node 22+ with pnpm (`corepack enable`), CMake 3.26+ and
Xcode on an Apple Silicon Mac.

```sh
git clone <repo-url> && cd ghi
git submodule update --init                  # not --recursive
pnpm install && pnpm build
./tools/scripts/build-nemo.sh                # speech engines (Metal)
pnpm --filter @ghi/desktop tauri dev --features nemo
```

On first launch the app downloads its models (about 4 GB, checked by SHA-256)
from Hugging Face; nothing else goes online unless you ask for it.

To build the app bundle and a DMG locally (ad-hoc signed, for testing):
`tools/release/build-dmg.sh --adhoc`. More in
[CONTRIBUTING.md](CONTRIBUTING.md#development-setup).

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

## Security

Report vulnerabilities privately; see [SECURITY.md](SECURITY.md).

"Ghira" and its logo are not covered by the code license; see [TRADEMARKS.md](TRADEMARKS.md).
