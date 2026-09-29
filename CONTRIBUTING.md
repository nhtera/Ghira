# Contributing to Ghi

Thanks for helping. Ghi is Apache-2.0 licensed, and every contribution is made
under that license.

## Sign your commits (DCO)

We use the [Developer Certificate of Origin](https://developercertificate.org/)
instead of a CLA. Add a sign-off to every commit:

```sh
git commit -s -m "feat(store): add VN fold"
```

This appends `Signed-off-by: Your Name <you@example.com>`, matching your git
author. CI rejects pull requests with unsigned commits. To fix a branch:
`git rebase --signoff main`.

## Development setup

| Tool | Version |
|---|---|
| Rust | pinned in `rust-toolchain.toml` (rustup installs it) |
| Node.js | 22+ |
| pnpm | pinned in `package.json` (`corepack enable`) |
| CMake | 3.26+ (NeMo-Speech.cpp) |
| macOS | Xcode (Apple Silicon for Metal) |
| Windows | Visual Studio 2022 Build Tools (C++), WebView2 |

```sh
git clone <repo-url> && cd ghi
git submodule update --init                  # not --recursive: NeMo-Speech.cpp nests large/LGPL repos we don't use
pnpm install
pnpm build                                   # frontend (needed before cargo builds the desktop crate)
cargo test --workspace
pnpm --filter @ghi/desktop tauri dev         # run the desktop app
./tools/scripts/nemo-smoke-build.sh          # optional: NeMo-Speech.cpp CPU build
```

Before opening a PR, run what CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
pnpm lint && pnpm typecheck && pnpm test
./tools/scripts/check-spdx.sh
./tools/scripts/check-net-egress.sh
cargo deny check licenses bans advisories sources
```

## Conventions

- **License header.** Every source file starts with
  `// SPDX-License-Identifier: Apache-2.0` (`#` for shell/Python).
- **Naming.** Rust: `snake_case`. TypeScript/JS files: `kebab-case`.
- **Commits.** [Conventional Commits](https://www.conventionalcommits.org/)
  (`feat:`, `fix:`, `docs:`...), signed off.
- **Typed commands.** Tauri commands use tauri-specta. After changing one, run
  `GHI_UPDATE_BINDINGS=1 cargo test -p ghi-desktop` and commit
  `apps/desktop/src/bindings.ts`.

## Rules that CI enforces

- **Network access only in `crates/ghi-net`.** No HTTP clients or sockets anywhere
  else, and no `fetch`/`WebSocket` in the webview. The app's CSP allows no
  remote origins. See [PRIVACY.md](PRIVACY.md).
- **No telemetry**, analytics or remote fonts/assets.
- **Dependency licenses.** Allowed: Apache-2.0, MIT, BSD-2/3-Clause, ISC,
  Zlib, MPL-2.0, OFL-1.1, Unicode-3.0, CC0-1.0. Not allowed in shipped binaries:
  GPL, AGPL, SSPL, LGPL (static), non-commercial or "source-available". No
  bundled FFmpeg.
- **No models or recordings in git.** Models are downloaded at a pinned
  revision and SHA-256. Eval recordings are private and never committed.

## Reporting security issues

Don't open a public issue. See [SECURITY.md](SECURITY.md).

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
