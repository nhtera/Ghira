# Security policy

## Reporting a vulnerability

Please **don't open a public issue**. Use GitHub's private vulnerability
reporting: **Security → Report a vulnerability** on this repository. We aim to
acknowledge reports within 3 working days and to agree on a disclosure date
with you.

In scope: the desktop and mobile apps, the Rust core, the update and model
download paths, LAN sync, and the release pipeline. Especially interesting:
any way user content (audio, transcripts, notes, voiceprints) could leave the
device without an explicit opt-in.

## Network policy

Ghira is offline-first; [PRIVACY.md](PRIVACY.md) defines exactly what may leave
the device. In code:

- `crates/ghi-net` is the **only** place allowed to open network connections.
  CI rejects HTTP clients and sockets anywhere else (`deny.toml`,
  `tools/scripts/check-net-egress.sh`).
- The webview's Content Security Policy allows no remote origins, forms,
  frames or `<base>` changes; the UI talks to Rust over IPC only. A CI test
  checks that the policy blocks remote images, fetches, sockets and form posts.
- Top-level navigation, which CSP does not cover, is limited to the app's own
  origin by a navigation guard, and `window.open` is denied.
- No telemetry.

## Supported versions

Before v1.0, only the latest release gets security fixes.

## Release integrity

- Releases are built only from **signed version tags** whose tag object
  matches the tag name and the commit being built, on `main`, from a clean
  checkout with `--locked` dependencies and no build caches. Pull requests
  never run the release workflow.
- The binding controls are repository settings: a tag ruleset limits who may
  create or move `v*` tags, and the protected `release` environment only
  accepts `v*` tags and requires a reviewer. Signing secrets are added only
  once that environment can require reviewers.
- macOS builds will be signed with an Apple Developer ID and notarized; Windows
  builds with an Open Source code-signing certificate (from the first alpha on).
- Models are downloaded at a pinned revision and verified by SHA-256.

## Updater key

The in-app updater only installs updates whose manifest is signed with the
project's Ed25519 updater key. The public key is compiled into the app.

**Custody.**
- The private key is generated **offline** and kept on a hardware token or an
  offline machine. It is never stored in CI or in any online secret store.
- Signing an update manifest needs **two people**: the owner signs, and a
  second maintainer checks the manifest (version, hashes, URLs) before it is
  published.
- An encrypted backup is kept offline in a separate place.

**Planned rotation** (e.g. yearly, or when a maintainer with access leaves):
1. Generate the new key pair offline.
2. Ship release N with **both** public keys trusted, signed with the old key.
3. Once most users run N or later (check download stats of N), sign releases
   with the new key only, and ship N+1 that trusts only the new key.
4. Destroy the old private key and record the rotation in the release notes.

**If the key is lost or compromised:**
1. Take the update feed offline so no further manifests are served.
2. Publish a security advisory with the fingerprint of the affected key.
3. If the old key is still under our control, ship one last update signed with
   it that trusts only a new key. Otherwise, users must download and install
   the new build manually from the official site, and we say so clearly.
