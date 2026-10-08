# Install from source

Ghira is pre-release. There is no signed download yet, so you build the Mac app
yourself. This page takes you from a clean Mac to a running app.

## What you need

- A Mac with Apple Silicon, running macOS 14.2 or later.
- Xcode, with its command line tools.
- Rust, installed with [rustup](https://rustup.rs). The repository pins the
  version it needs, and rustup installs it for you.
- Node.js 22 or later, with pnpm (`corepack enable` turns it on).
- CMake 3.26 or later.
- About 4 GB of free disk space for the models, plus room for the build.

## Build and run

<!-- quickstart -->

Here is what the commands do:

1. **Clone the repository.** The speech engine lives in a git submodule. Run
   `git submodule update --init` without `--recursive`: the engine nests large
   repositories Ghira does not use.
2. **Build the interface.** `pnpm install && pnpm build` builds the web
   interface the desktop app shows.
3. **Build the speech engines.** `./tools/scripts/build-nemo.sh` builds
   NeMo-Speech.cpp with Metal. It takes a while the first time.
4. **Start the app.** `pnpm --filter @ghi/desktop tauri dev --features nemo`
   builds and opens Ghira.

## First launch

Ghira walks you through setup. The step that matters here is **Speech models**:
the app downloads the models for your Mac's preset (about 3.4 GB on Light, about
4 GB on Balanced and Max) from Hugging Face, and checks each file before it uses
it. You can keep going while they download. See [Speech models](models.md) for
what is downloaded and how to install the files without a network.

[Record your first meeting](getting-started.md) covers the rest of the setup.

## Fetch models for development

If you work on the code, you can fetch the same pinned models into `./models`
without the app:

```sh
./tools/scripts/fetch-models.sh                  # all the standard models
./tools/scripts/fetch-models.sh nemotron-3.5-asr # or name the ones you want
```

The model ids are in `crates/ghi-models/registry.toml`. Models marked optional,
such as the Whisper models, are skipped unless you name them.

## A development build is not a release build

`tauri dev` makes a debug build. A debug build keeps the encryption key for your
meetings in a file next to the data folder, not in the Keychain. Do not keep
real meetings in a debug build. To use the Keychain in a debug build, set
`GHI_KEYSTORE=keychain` before you start it.

To build an app bundle and a disk image for testing, run
`tools/release/build-dmg.sh --adhoc`. The result is signed ad hoc, so macOS
treats it as an unknown developer's app.

## Troubleshooting

- **A `cargo` build of the desktop app fails.** Run `pnpm build` first. The
  desktop crate needs the built interface.
- **A submodule folder is empty.** Run `git submodule update --init` from the
  repository root.
- **`build-nemo.sh` fails.** Check that CMake is 3.26 or later (`cmake
  --version`) and that Xcode's command line tools are installed
  (`xcode-select --install`).
- **The app opens but there is no live transcript.** The speech models are not
  installed yet, or you started the app without `--features nemo`. Ghira still
  records, and it processes the recording once the models are installed.
- **The models do not download.** Check that Strict offline is off in
  **Settings → Privacy**. The download resumes where it stopped.
- **macOS asks for the microphone or system audio.** Allow both. The microphone
  hears you and the room. **Screen & System Audio Recording** hears the other
  people on a call. Without it, Ghira records your microphone only.

## Limits

- Only macOS on Apple Silicon is supported. Windows is not shipped, and there
  is no Android app.
- There is no automatic update in a build from source. To update, pull the
  repository and build again.
