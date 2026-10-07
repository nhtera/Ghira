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
./tools/scripts/check-windows.sh   # Windows type-check + clippy from a Mac (mingw-w64 GNU target; not MSVC)
cargo deny check licenses bans advisories sources
# never `cargo clean` / `rm -rf target` (shared by agents and the dev app; disk is tight): use `cargo clean -p <crate>`
# speech engines (phase 3; needs tools/scripts/build-nemo.sh and fetch-models.sh first):
cargo clippy -p ghi-speech -p ghi-cli --all-targets --features ghi-cli/nemo -- -D warnings
cargo test -p ghi-speech -p ghi-cli --features ghi-cli/nemo
cargo test -p ghi-desktop --features nemo -- --ignored real_models --nocapture  # real-model Core harness (minutes; needs cargo build -p ghi-llm-worker; skips without models)
cargo test -p ghi-core --features nemo --release --test live_bench -- --ignored --nocapture  # live transcript bench: caption lag, line length, WER (real time, ~3 min/set; GHI_LIVE_BENCH_SPEED=0 for WER only, one run at a time; sets from fetch_public_sets.py --sets fleurs-vi,fleurs-en,ami-sdm,ami-text,voxconverse,vimedcss,earnings21,vietmed)
# Whisper final pass (optional; needs tools/scripts/build-whisper.sh and fetch-models.sh whisper-large-v3-turbo silero-vad):
cargo clippy -p ghi-speech -p ghi-core -p ghi-cli --all-targets --features ghi-cli/nemo,ghi-cli/whisper -- -D warnings
cargo test -p ghi-speech --features whisper && cargo test -p ghi-core --features nemo,whisper --test whisper_real -- --ignored --nocapture  # NeMo + Whisper in one process
ghi transcribe x.wav --asr whisper --lang vi --pass final   # CLI; the app: setting asr_final = "whisper" (debug builds: GHI_ASR_FINAL=whisper), build with ghi-desktop --features nemo,whisper; fetch-models.sh skips `optional` models: name them
# speaker embedder (phase 14c; parity tests skip without fetch-models.sh campplus-zh-en):
cargo clippy -p ghi-speech --features voice --all-targets -- -D warnings
cargo test -p ghi-speech --features voice
cargo clippy -p ghi-core -p ghi-cli --features ghi-cli/voice --all-targets -- -D warnings
cargo test -p ghi-core -p ghi-cli --features ghi-cli/voice
# iOS app (phases 7, 16; needs Xcode + xcodegen; simulator only):
cargo clippy -p ghi-mobile --features nemo --target aarch64-apple-ios-sim -- -D warnings
cargo test -p ghi-mobile --lib
pnpm --filter @ghi/mobile typecheck && pnpm --filter @ghi/mobile lint && pnpm --filter @ghi/mobile test
pnpm --filter @ghi/mobile test:e2e   # WebKit at iPhone size; PORT=<n> for a private server
apps/mobile/scripts/build-ios.sh --sim --test-hooks   # (any build without --test-hooks checks itself with check-no-test-hooks.sh)
apps/mobile/scripts/sim.sh boot && apps/mobile/scripts/sim.sh install && apps/mobile/scripts/sim.sh grant
apps/mobile/scripts/test-ios-sim.sh -p ghi-store -p ghi-core -p ghi-mobile   # needs the booted simulator
native/ios/GhiUITests/run.sh         # lifecycle + smoke XCUITests, on the installed app (GHI_REAL_ENGINES=1: real models)
native/ios/GhiUITests/flows.sh       # resets, installs, grants, then the full flow with scripted engines (no models)
# on the owner's real iPhone (only when asked; test-hooks app installed: build-ios.sh --test-hooks, devicectl install):
GHI_DEVICE_UDID=<xcodebuild udid> GHI_DEVICE_CTL_ID=<devicectl id> GHI_FAKE_MIC_PATH=<wav> GHI_REAL_ENGINES=1 native/ios/GhiUITests/run.sh
apps/mobile/scripts/selftest-ios.sh <16k wav>   # on-device ASR throughput + text (test-hooks build; models via push-models.sh after one launch)
./tools/scripts/check-no-test-hooks.sh <release libghi_mobile_lib.a | Ghira.app>   # --expect-hooks on a hooked build
pnpm gen:licenses && pnpm gen:licenses:mobile   # About -> Licenses data; CI fails on a diff (license.yml)
# CI picks the simulator with GHI_SIM_UDID (sim.sh create); locally the default is iPhone 17 Pro, iOS 26.3.
# LAN sync verification (phase 15, 15-L): the in-memory tests (convergence, mitm, lease race, delete -> undecryptable,
# retention, audio resume, trigger guard) run with `cargo test --workspace`; PROPTEST_CASES=300 raises the convergence cases (default 32).
# The socket tests (real listener on a private address + `lsof -a`, the `ghi sync serve` process) skip with a message unless
# GHI_SYNC_LAN_IP is a private address of this Mac (there is no loopback bypass):
GHI_SYNC_LAN_IP=$(ipconfig getifaddr en0) cargo test -p ghi-sync --test listener_audit -- --nocapture   # needs `cargo build -p ghi-cli` for the process test
(cd crates/ghi-sync && cargo +nightly fuzz run record fuzz/corpus/record fuzz/seeds/record -- -max_total_time=60)   # also frame, track_pages, and message/qr with their fuzz/seeds/<target>; standalone crate, CI only type-checks it
cargo test -p ghi-store --release --test sync_trigger_cost -- --ignored --nocapture   # bench: cost of the sync_log triggers on segment inserts
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
`ghi-store` `organize.rs` (phase 14d, migration 0007): folders, tags and
`meeting_tags` (names unique by folded form, link rows with fresh gids),
`meetings.source_app`, sealed `calendar_ct` / `track_speakers_ct`, and
`segments.overlap` via `mark_overlaps`.
`crates/ghi-app` (phase 16-A): the app layer both Ghira apps share. `Core` and the
store-facing Tauri commands (library, detail, speakers, cloud, settings,
lock, audio protocol, import queue, voice) live there, registered by the desktop
with `ghi_app::<module>::<cmd>` (so bindings are the same for any app);
windows, dialogs, tray, menu, calendar, updater stay in `apps/desktop`, whose
files of the same names `pub use ghi_app::…` (features `local-llm`,
`embeddings`, `nemo`, `voice`, `specta`).
Phase 14d modules (stubbed in W0-B, filled by slices S2-S6): `ghi-core` `calendar`
(events, ICS, prompt rules), `activity` (per-track speech spans), `presets`
(import source/title/date, Zoom grouping), `recluster` (>8 speakers),
`import::import_tracks`, `vocab::meeting_terms`; desktop `calendar_cmd`,
`calendar_mac` (EventKit), `organize_cmd` (folders/tags); UI mount points under
`features/{calendar,folders}`; mocks `ipc/mock-{calendar,organize}.ts`.
Import (S2/S7): `import_tracks` mixes a Zoom recording's per-participant files into
one `file` track and stores each participant's speech spans (`track_speakers`);
the final pass uses them instead of the diarizer. Staging groups the tracks
(`import_cmd.rs`, `presets::zoom_group`); `ghi import --tracks <dir|files>`.
Calendar (S4/S5): events are read on demand (EventKit on macOS, one ICS file on
any OS; its path stays in Rust, setting `calendar`) and never stored; a ticker
(`calendar_cmd::spawn_ticker`, beside detection) offers "record when it starts"
once per event (shared with the app detector), and only a recorded meeting
keeps sealed `calendar_ct` (attendees feed vocab, cloud redaction, rename
suggestions and the notes template). Needs the Calendars entitlement + plist key.

Notes engine: `crates/ghi-llm` (templates in `templates/*.toml`, generated
JSON schemas, map-reduce notes with citations, enhance, Ask, redaction, send
preview, cloud providers) + `crates/ghi-llm-worker` (llama.cpp over stdio, a
separate process); `ghi notes|ask|keys` and `ghi store notes`. Local model:
`tools/scripts/fetch-models.sh qwen3-4b`, then `cargo build -p ghi-llm-worker`
(the golden tests skip without both). Cloud sends only go through `ghi-net`'s
`CloudGrant`, bound to the exact previewed bytes.

Core pipeline (phase 8): `crates/ghi-core` — `session` (capture → pump →
bundles; ASR ring → `live` engine → `persist` → store; event bus `events`),
`speakers`/`aligner` (arrival order, provisional, Me in call mode),
discard [RT-1] (store transaction + bundle rotation), `jobs` (the job runner;
recording preempts), `final_pass` (v2 + `carry` name carry-over + `vocab`),
`notes_job`, `import`, `recover` (startup). Without speech models a
session records only and its jobs wait until the models are installed
(`JobHandler::ready`). Speech engines behind
`engines::SpeechEngines` (NeMo with `nemo`, a scripted fake in tests).
`ghi session|jobs|import`; the desktop has a thin command layer (`core.rs`)
and the typed `coreEvent`.

Desktop UI (phase 9): `packages/ui` (`@ghi/ui`: tokens from `tokens.json` →
committed `tokens.css` via `gen:tokens`; bundled fonts `gen:fonts`; icons
`gen:icons` into `icon-data.ts` (Material on mac, Fluent on Windows; notices in
`THIRD_PARTY_NOTICES.md`); Radix-based primitives; the brief §7 components with
`*.stories.tsx`), the story gallery (`pnpm --filter @ghi/ui gallery`), and
`packages/i18n` (typed i18next keys, EN/VI, product name via `{{app}}`; the
copy was extracted once from the design by `scripts/extract-from-design.mjs`).
`apps/desktop/src`: TanStack Router (hash history), `ipc/` (tauri-specta
commands/events; a scripted mock outside Tauri), Zustand live store, shell
(sidebar, ⌘K palette, keyboard map; the macOS menu in `src-tauri/src/menu.rs`
owns ⌘⇧R/⌘M/⌘K/⌘,). UI copy only from locale files (lint), text nodes only
(RT-6). e2e also runs the gallery under the prod CSP, axe on every story, and
macOS WebKit visual baselines (`--update-snapshots` after intended changes).

Review flows (phase 11): desktop commands in `src-tauri/src/{detail,export_cmd,
import_cmd,cloud_cmd,settings_cmd,dialogs}.rs`; full-meeting playback is a
chunked WAV over `ghi-audio://` play tokens (`audio_protocol.rs`), waveforms
cached sealed in the store (schema v4). Exports are rendered in
`ghi-core/src/export.rs`; cloud plan → exact-bytes preview → send in
`ghi-core/src/cloud.rs`; follow-up email in `ghi-core/src/email.rs`. Native
dialogs are `rfd` in Rust: the webview never sees a file path. The UI uses the
scripted mock (`ipc/mock*.ts`; `window.__ghiMock` hooks) for tests.

Release (phase 12): `tools/release/stage-bundle.sh` (worker sidecar + NeMo
dylibs into the bundle), `build-dmg.sh [--adhoc]`, `sign-manifest.sh`
(owner, offline), `crash-safety.sh`, `net-audit.sh`, `mirror-models.sh`,
`offline-models.sh`; `apps/desktop/src-tauri/tauri.release.conf.json` +
`entitlements.plist`; `release.yml` signs only when Apple secrets exist.
Updater: `crates/ghi-update` (minisign manifest, no downgrade, Team-ID check;
inert until `FEED_URL` + `PUBLIC_KEYS` are set). Local crash reports + event
log: `crates/ghi-diag` (no meeting content; tested). Acceptance:
`tools/eval` `ghi-eval acceptance`. Checklists in `docs/release/`.

P1 features (phase 14): app lock in `src-tauri/src/lock_cmd.rs` (LocalAuthentication;
while locked `Core::store()` refuses and content events are gated; recording,
jobs and imports use `store_even_locked`). Semantic search: chunk embeddings
(`ghi-llm/src/embed.rs`, Qwen3-Embedding via the worker) sealed per meeting
(`ghi-store` embeddings, `index_gen` re-index on edits), the `embed_index` job
(`ghi-core/src/index_job.rs`, Balanced/Max only), hybrid FTS + vector RRF and
Ask across meetings in `ghi-core/src/ask_all.rs` + `src-tauri/src/ask_cmd.rs`.
People and voice: `ghi-store` `people.rs`/`voice.rs` (persons linked by name,
per-profile keys, crypto-shred), speaker embeddings in `ghi-speech` feature
`voice` (CAM++ via tract), matching in `ghi-core` `profiles.rs`/`voice_step.rs`
(final pass) and `voice_job.rs`; desktop `people_cmd.rs`/`voice_cmd.rs`.
Third-party voice profiles are hard-off (`THIRD_PARTY_APPROVED` in `system.rs`,
store `ThirdPartyApproved` token) until counsel signs off.

iOS app (phase 7 spike, phase 16 app; Simulator-verified, device runs are owner items):
`crates/ghi-app` is the shared core of both apps; `apps/mobile` (Tauri 2, iOS only)
mounts it with the phone's own `src-tauri/src/cmd/*` (record, meetings, onboarding,
privacy, settings, voice, models, import, lifecycle, events), the recording engine
(`session`, `backlog`, `gate`, `engine`, `tier`, `lifecycle`, `inbox`) and the
`ghi-core` lifecycle jobs. UI: `apps/mobile/src` (TanStack Router, hash history;
screens M1-M6 under `features/*`, the `@ghi/ui` iOS primitives TabBar/NavBar/List/
PhoneButton/Sheet; copy in `packages/i18n/locales/mobile/*.json`, EN + VI; the
scripted mock `ipc/mock*.ts` outside Tauri). `native/ios`: `GhiAudio` (Swift audio,
lifecycle, Live Activity, C ABI in `include/ghi_ios.h`), `GhiLiveActivity`,
`GhiShareExtension` (inbox in the App Group), `GhiUITests` (XCUITest). Data is
Application Support, encrypted store + bundles; no `UIFileSharingEnabled`.
Calendar on iOS: `native/ios/GhiAudio/GhiCalendar.swift` (EventKit, JSON over the C ABI) and
`src-tauri/src/cmd/calendar.rs` (setting `calendar_phone`, read on demand, 60 s memory
cache, names a meeting at record start; the EventKit → event conversion is shared in
`ghi-core` `calendar::from_raw`, `meeting_attendees`/`meeting_contacts` in `ghi-app`).
Test hooks (`GHI_FAKE_MIC`, `GHI_FAKE_ENGINES`, `com.nhtera.ghira.test.*` Darwin
notifications) exist only with the cargo feature `test-hooks` + Swift `GHI_TEST_HOOKS`
(`build-ios.sh --sim --test-hooks`); `tools/scripts/check-no-test-hooks.sh` guards
release artifacts. Build/install: `apps/mobile/scripts/build-ios.sh [--release|--sim
[--test-hooks]]` (the Xcode project is generated from the committed
`gen/apple/project.yml`; don't re-run `tauri ios init`; the signing team comes from
`$APPLE_DEVELOPMENT_TEAM`, never committed), `sim.sh` (boot/install/grant/models/ui),
`test-ios-sim.sh` (Rust tests ON the simulator), `push-models.sh` and `selftest-ios.sh`
(device). NeMo-Speech.cpp for iOS: `tools/scripts/build-nemo-ios.sh` (XCFrameworks in
`target/nemo-ios`). Never touch a connected iPhone from automation. The About
licenses are generated: `pnpm gen:licenses:mobile` (`src/generated/licenses.json`).

LAN sync (phase 15; spec `Plans/docs/07-sync-protocol.md`, plan `phase-15-plan.md`): desktop = hub,
phone = spoke, star topology. `crates/ghi-sync`: Noise `IKpsk2` over TCP (`snow`; QR pairing payload
`GHI1:`+base45 CBOR, one-time 120 s PSK, then a pinned pair PSK), CBOR wire messages, hub/spoke sessions
(push then pull, tombstones first), resumable verified audio (phone → desktop, `bundle::RawImport`), leases
for the desktop final pass (epochs, sleep-inclusive clock, fence in the `ghi-core` job runner), unpair/wipe
control, `service` (HubNode, pair/run helpers) and the GHIX "Export for another device" fallback. Sockets and
mDNS only in `ghi-net::lan` (`is_lan` on bind/accept/connect, no 0.0.0.0, `listeners_open()`; mDNS feature
`mdns` desktop/CLI only; iOS uses NWBrowser in `GhiSync.swift`). Store side `ghi-store/src/sync/` (feed via
`sync_log` triggers, devices incl. 'known' relay origins, keys, leases, merge engine `apply.rs`/`rules.rs`,
parked rows, conflict copies, folder/tag folds); migration 0009. App: `ghi-app` `sync_service.rs` (hub +
listener lifecycle, closed when locked) and `sync_spoke.rs` (phone loop, lease grantor); `ghi sync
serve|pair|run|status|export|import`. Voice profiles never sync in v1.

Speech engines: `crates/ghi-speech` (NeMo-Speech.cpp FFI, feature `nemo`); models
pinned in `crates/ghi-models/registry.toml`; decision record `Plans/docs/06`.
Optional final-pass ASR (feature `whisper`, desktop + CLI, not mobile): Whisper
large-v3-turbo q5 + Silero VAD over a pinned `third_party/whisper.cpp` built STATIC
(`tools/scripts/build-whisper.sh`; never shared, NeMo ships its own patched dynamic ggml).
`ghi-speech/src/whisper`: `shim.c` (flat C face; no hand-written whisper structs),
`plan.rs` (VAD grouping <= 29 s, DTW words, hallucination guard; model-free tests),
`WhisperFinalEngines` in `ghi-core` (Whisper reads, NeMo diarizes; what the guard drops is read by a lazily loaded Nemotron, `WhisperStream::with_fallback`), picked in
`ghi-app` `final_engines()` when store setting `asr_final` = `whisper` (default `nemo`;
`GHI_ASR_FINAL` overrides in debug builds; no settings UI yet) and both models are installed,
else (or on any load failure) NeMo. Preemption aborts a decode mid-window (`AsrStream::finish_abortable`);
`./tools/scripts/check-no-ggml-export.sh <binary>` guards that ggml stays private.

Eval kit: `tools/eval` (`ghi-eval`, Python 3.11 + uv). The data, `ghi` CLI and
report contract is `tools/eval/docs/formats.md`. Recordings, transcripts and
run directories never enter git; only aggregate reports leave the customer.
