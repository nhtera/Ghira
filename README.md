# Ghira

Ghira is an offline-first AI meeting note taker for English and Vietnamese, with
live speaker diarization. Audio, transcripts and notes stay on your device.
Cloud AI is optional, per meeting, and sends transcript text only, never audio.

> Status: **pre-release.** The macOS desktop app (Apple Silicon, macOS 14.2+) is
> the primary target. The iOS app is built and tested on the Simulator only;
> runs on a physical iPhone are still owner checks. **Windows is not shipped**
> (the code is only type-checked and linted for Windows from a Mac), and there
> is no Android app. There is no signed download yet; you can build from source.

## What it does

- **Record** a call (your mic + the meeting app's audio) or a room, with a live
  transcript in English, Vietnamese or both, and speakers told apart as they talk.
- **Notes** written on your Mac by a local model: summary, decisions, action
  items with owners, open questions, each sentence linked to the moment it was
  said. Your own notes are kept and expanded.
- **Review**: search every meeting (accents optional), play any cited moment,
  correct the transcript, rename speakers, export to Markdown, Word, text,
  subtitles or Obsidian, draft a follow-up email.
- **Encrypted store**: meetings live in an encrypted local database with a key per
  meeting; deleting a meeting destroys its key.
- **Import** recordings (Voice Memos, Zoom, Plaud, any audio or video file).
- **Optional cloud AI**, per meeting: you see the exact text before anything is
  sent, names can be hidden, audio never leaves the device.

## Quick start (from source, macOS)

Needs Rust (rustup), Node 22+ with pnpm (`corepack enable`), CMake 3.26+ and
Xcode on an Apple Silicon Mac.

```sh
git clone https://github.com/nhtera/Ghira.git && cd Ghira
git submodule update --init                  # not --recursive
pnpm install && pnpm build
./tools/scripts/build-nemo.sh                # speech engines (Metal)
pnpm --filter @ghi/desktop tauri dev --features nemo
```

On first launch the app downloads the models for the chosen hardware preset
(a few GB, pinned and checked by SHA-256) from Hugging Face; nothing else goes online unless you ask
for it. For development, `./tools/scripts/fetch-models.sh [model-id ...]` fetches
the same pinned models into `./models` (the ids are in
`crates/ghi-models/registry.toml`).

To build the app bundle and a DMG locally (ad-hoc signed, for testing):
`tools/release/build-dmg.sh --adhoc`. More in
[CONTRIBUTING.md](CONTRIBUTING.md#development-setup).

## Checks

What CI runs (the full list, including iOS and the speech-engine features, is in
[CLAUDE.md](CLAUDE.md#checks)):

```sh
pnpm build && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
pnpm lint && pnpm typecheck && pnpm test && pnpm --filter @ghi/desktop test:e2e
./tools/scripts/check-spdx.sh && ./tools/scripts/check-net-egress.sh
cargo deny check licenses bans advisories sources
```

## Principles

- **Offline by default.** No telemetry. All network I/O goes through one crate (`crates/ghi-net`); the webview never touches the network. See [PRIVACY.md](PRIVACY.md) for exactly what may leave your device.
- **Open source**, [Apache-2.0](LICENSE). Contributions use Conventional Commits and an SPDX header on every source file; see [CONTRIBUTING.md](CONTRIBUTING.md).
- **Your voice is yours.** Voice profiles need explicit consent. Cloud AI is opt-in per meeting and sends transcript text only.

## Repository layout

| Path | What |
|---|---|
| `crates/` | Rust core (`ghi-*`; "Ghi" is the codename): audio capture, speech, store, core pipeline, LLM client and worker, network policy, shared app layer, CLI |
| `apps/desktop/` | Tauri 2 desktop app (React + TypeScript) |
| `apps/mobile/` | Tauri 2 iOS app (React + TypeScript) |
| `native/macos/`, `native/ios/` | Swift audio and platform code (`native/android/` is empty) |
| `packages/` | `@ghi/ui` components and `@ghi/i18n` translations (EN, VI) |
| `third_party/NeMo-Speech.cpp` | NVIDIA speech runtime (git submodule) |
| `tools/` | `eval/` (evaluation kit), `release/` (bundle, DMG, audits), `scripts/` (checks, model fetch, builds) |
| `docs/` | Release notes, checklists and audits; see [docs/README.md](docs/README.md) |

## Security

Report vulnerabilities privately; see [SECURITY.md](SECURITY.md).

## License

[Apache-2.0](LICENSE). Third-party components and their licenses are listed in
[NOTICE](NOTICE), [THIRD_PARTY_NOTICES.html](THIRD_PARTY_NOTICES.html) and
[third_party/NATIVE_NOTICES.md](third_party/NATIVE_NOTICES.md).

"Ghira" and its logo are not covered by the code license; see [TRADEMARKS.md](TRADEMARKS.md).
