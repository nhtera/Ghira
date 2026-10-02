# Security verification (phase 12 step 3)

Verification of controls that earlier phases **designed**, not new design [RT-12]. Method: read
the code, run the existing tests and scripts, record evidence and gaps. Verified 2026-10-02 on
macOS 15, commit `892c56f` plus the uncommitted working tree of that day. A row is **Verified**
only if a test or script I ran backs it; **Partly** if code reading backs it but something needs
the signed app; **TODO** if the control is still arriving.

## Summary

| # | Control (phase 12 step 3) | Status | Gaps |
|---|---|---|---|
| 1 | CSP + safe rendering | Verified | F3 (style-src), F4 (freezePrototype) |
| 2 | Key binding + dev-key exclusion | Dev-key exclusion Verified; key binding **Partly** | F2 |
| 3 | Crypto-shred + backup exclusion | Verified | F1 (fixed) |
| 4 | NetGuard / strict offline | Verified | none |
| 5 | Audio protocol tokens | Verified | none |
| 6 | Redaction + CloudGrant | Verified | F6 (redaction is best effort by design) |
| 7 | Model hash checks | Verified | F5 |
| 8 | Crash-report scrubbing | Verified (unit + end-to-end tests) | owner: check a real `.ips` copy |
| 9 | Updater | Verified in tests; inert until feed + key | owner: alpha.1 → alpha.2 on /Applications |

## Findings

| ID | Severity | Finding | Recommendation | Owner |
|---|---|---|---|---|
| F1 | Fixed (tests now base on today's schema) | `cargo test -p ghi-store` fails: `snapshot_restore.rs` `snapshots_exist_during_a_migration_and_are_deleted_after_success`, `a_snapshot_restores_the_pre_migration_database`, `snapshots_are_encrypted_with_the_same_key` panic in `common::meeting` (`tests/common/mod.rs:75`) with `table meetings has no column named created_at`. They open the store with `MIGRATIONS[..1]` (schema v1) and then call `create_meeting`, which since phase 11a (`1a70354`, `store.rs:630`) inserts `created_at`, a column added by migration 0004. A test-fixture break, not a product bug | In those three tests create the pre-upgrade meeting with SQL valid for v1, or open with the full `MIGRATIONS` and a failing extra migration on top. Until then the snapshot-restore control is verified only by `migration_failure.rs` (5 tests pass) | store owner |
| F2 | Medium (design decision) | The desktop app's master key is stored with `KeychainStore::new("com.nhtera.ghira", "store")` (`apps/desktop/src-tauri/src/core.rs:71-76`): the **login** keychain, legacy ACL, not `ThisDeviceOnly`. `KeychainStore::data_protection` (`keys/apple.rs:92`, "for the signed app": data-protection keychain, `WhenUnlockedThisDeviceOnly`) is never called, and app lock (`Protection { app_lock }`, user presence) is not wired in the desktop. The doc comment of `apple.rs:21` says the signed app uses `data_protection`. A Developer-ID app needs `keychain-access-groups` plus an embedded provisioning profile for that keychain; `entitlements.plist` has only `audio-input` | Decide and record it. Either adopt `data_protection` (check a provisioning profile is obtainable for a Developer ID app) or document in `SECURITY.md` that the key is a login-keychain item readable by other same-user processes after a user-approved prompt, and fix the comment. Verify on the signed build (owner list below) that only the signed Ghira reads the item | owner / store owner |
| F3 | Low (accepted, tested) | `style-src 'unsafe-inline'` (`tauri.conf.json:16`), needed by Radix scroll lock. Cannot exfiltrate: `img-src`, `font-src` (falls to `default-src 'self'`), `connect-src` are local; `csp.test.ts` pins that nothing else allows `'unsafe-inline'` and that `index.html` has no inline `<style>` | None; keep the test | |
| F4 | Low | `app.security.freezePrototype` is not set (`tauri.conf.json`, default off). Freezing `Object.prototype` is free hardening against prototype pollution through a dependency | Set `"freezePrototype": true` and run the e2e suite | desktop owner |
| F5 | Low | `verify_for_load` (`ghi-models/src/verify.rs:211`) hashes the file, then native code opens it by path: a hash-then-load window, and the per-process cache key is `(path, size, mtime)`, which a same-user process can forge. Also, the CLI's explicit `--asr-model/--diar-model` paths skip verification (`ghi-cli/src/cmd/session.rs:100-108`, a dev flag; the CLI is not in the DMG) | Accept: a same-user attacker can already replace the app's data. If wanted later: hash the opened file descriptor and load from it | none |
| F6 | Info | Redaction (`ghi-llm/src/redact.rs`) removes emails, URLs, numbers/IDs, spoken digits and the names it is told about; it cannot know every sensitive word. The control that matters is the preview (exact bytes + SHA-256 + leftover warnings) and the per-meeting opt-in and lock | Keep the preview mandatory (it is: the grant binds the previewed bytes) | |
| F7 | Info | `NetPolicy::permits` logs every decision with the host (`ghi-net/src/lib.rs:56`, `log::info!`). With the 12c rolling log this puts model-mirror, update and provider hostnames, and LAN peer hostnames/IPs, in a local file | Confirm in 12c review that the log stays local, is excluded from the crash report scrub whitelist where needed, and holds no URL path or query | crash-12c |

## 1. CSP and safe rendering

| Check | Evidence | Result |
|---|---|---|
| Production CSP is local only: `default-src 'self'`; `object-src`, `form-action`, `base-uri`, `frame-src` = `'none'`; `connect-src ipc: http://ipc.localhost`; media/img only `ghi-audio:` and `data:` | `apps/desktop/src-tauri/tauri.conf.json:16`; `tests/unit/csp.test.ts` (5 tests: defaults, local sources only, no inline/eval script, `unsafe-inline` only in `style-src`, no inline `<style>`). Ran `pnpm exec vitest run tests/unit/csp.test.ts`: 5 passed | Verified |
| CSP blocks remote img, fetch, WebSocket, form posts and stylesheets in real engines | `tests/e2e/csp.spec.ts:8,59`. Ran `playwright test csp.spec.ts --project=chromium --project=webkit`: 4 passed. `gallery-csp.spec.ts` runs every story under the CSP | Verified |
| Top-level navigation (not covered by CSP) is confined to the app origin | `src-tauri/src/navigation.rs:10-27`, tests `app_origins_are_allowed`, `remote_and_lookalike_origins_are_blocked`, `dev_server_only_in_debug` (ran `cargo test -p ghi-desktop --lib -- navigation`: 3 passed); plugin registered first at `lib.rs:409` | Verified |
| Webview does no network I/O; text nodes only | `eslint.shared.mjs` `noNetwork` (fetch, XHR, WebSocket, EventSource, RTCPeerConnection, `window.open`, sendBeacon) and `safeRendering` (`dangerouslySetInnerHTML`, `innerHTML`/`outerHTML`, markdown/HTML libraries) applied in `apps/desktop/eslint.config.js:17-18` and `packages/ui/eslint.config.js:17-18`. `pnpm --filter @ghi/desktop lint`: 0 errors. Source scan for `innerHTML`, `insertAdjacentHTML`, `eval(`, `new Function`, `<a href`, `window.open`: none | Verified |
| No runtime CDN, fonts or scripts from remote origins | `apps/desktop/dist` and the release binary: the only `http(s)://` strings in the frontend bundle are licence texts and XML namespaces; `strings target/release/ghi-desktop` shows only the provider hosts (`api.openai.com`, `api.anthropic.com`, `generativelanguage.googleapis.com`), `huggingface.co`, `github.com` (update host) | Verified |
| Tauri capabilities are minimal | `capabilities/default.json` (`main` only): `core:event`, window basics and Ghira's own `allow-*` commands; no shell, http, fs or opener plugin; `detect`, `mini`, `popover` windows have 7-12 permissions and none of the data commands. Shell-outs (`system.rs:344`, `dialogs.rs:101`, `diag_cmd.rs:181`) take fixed URLs or a path the Rust side stored (`LastExport`), never a path from the webview; imports take native-dialog/drop paths by id (`import_cmd.rs`) | Verified |
| Gaps | F3, F4 | |

## 2. Key binding and dev-key exclusion

| Check | Evidence | Result |
|---|---|---|
| Dev file key store exists only with `debug_assertions` and is marked | `ghi-store/src/keys/mod.rs:35` (`#[cfg(debug_assertions)]`), `keys/dev.rs:12` (`GHIRA-DEV-FILE-KEYSTORE-DO-NOT-SHIP`); desktop uses it only under `#[cfg(debug_assertions)]` and `GHI_KEYSTORE != keychain` (`core.rs:52-67`); CLI `keystore.rs:19-26` | Verified |
| Release binaries do not contain it | `tools/scripts/check-no-dev-key.sh target/release/ghi target/release/ghi-desktop`: `no dev key: ok`; the same script on `target/debug/ghi` fails (control works); CI runs it (`.github/workflows/ci.yml:100`) | Verified |
| Master key in the OS key store, crash-safe two-slot save, non-synchronizable | `keys/apple.rs:1-40`; `tests/keychain.rs` (4 tests: save/load/replace/delete, interrupted save never loses the newest ring, app-lock and data-protection fail clearly without entitlements), `keychain_secrets.rs`: all pass (`cargo test -p ghi-store`) | Verified for the login-keychain path |
| Key is bound to this device and to the signed app | The app uses the login keychain (`core.rs:71-76`), see F2; the data-protection / user-presence path is "verified in the signed app (phase 12)" per `apple.rs:12-13` and cannot be exercised here | **Partly** (F2) |
| Per-meeting DEKs wrapped by the master key, bound to the meeting | `keys::tests::dek_wrapping_is_bound_to_the_meeting`, `rotation_retires_old_wraps` (unit tests pass) | Verified |

## 3. Crypto-shred and backup exclusion

| Check | Evidence | Result |
|---|---|---|
| Delete destroys the meeting key first; audio, transcript, notes become undecryptable | `store.rs:1762-1810` (`delete_meeting`: begin wrap rotation, tombstones, `shred_key` zeroes `dek_wrapped`, files, rows, index entries); `tests/crypto_shred.rs`: `delete_makes_audio_transcript_and_notes_undecryptable` (:208), `delete_leaves_no_tokens_snapshots_or_wal_and_a_wrong_key_fails_cleanly` (:460), `pre_migration_snapshots_are_purged_by_delete` (:422), `delete_all_shreds_everything_and_rotates_the_key` (:435) | Verified |
| A crash at any step finishes on next open | `a_crash_after_the_key_is_shredded_finishes_on_next_open` (:380), `a_crash_between_the_delete_and_the_rotation_is_finished_on_open` (:659), `a_crash_in_the_middle_of_a_rotation_is_finished_on_open` (:567); all 16 crypto_shred tests pass | Verified |
| Older copies cannot unwrap a shredded key (rotation of the wrap secret) | `rotate_wraps` (`store.rs`), `recovery_file_is_rewritten_after_the_rotation`, `an_old_recovery_file_never_replaces_the_current_key` | Verified |
| Data directory is excluded from Time Machine / device backups on every open, default off | `store.rs:392-400` (`exclude_from_backup(dir, !include)` on open), `backup.rs` (`NSURLIsExcludedFromBackupKey`); `tests/backup.rs::set_read_back_and_unset` cross-checks with `tmutil isexcluded` on macOS: passes | Verified |
| Honest limit | A backup taken before a delete keeps the data, which is why the directory is excluded by default (`store.rs:1750`); the master key is in the Keychain, not in the data directory. APFS free blocks can survive: it is a crypto-shred, not a wipe | Documented |
| Migration safety (RT-12) | `tests/migration_failure.rs` (5 pass): failing Rust migration and SQL error roll back, damage a rollback cannot undo is repaired from the snapshot, retry works; `snapshot_restore.rs` 5 of 8 pass (F1). No CLI-level test added: a failing migration cannot be injected into the `ghi` binary without a test hook, and the store-level tests already drive the same `Store::open_with_migrations` path the CLI and app use | Verified at store level, F1 |

## 4. NetGuard and strict offline

| Check | Evidence | Result |
|---|---|---|
| Policy: strict offline allows only private LAN; default allows the allowlist and LAN; any other host is denied under every policy | `ghi-net/src/lib.rs:54-66`; tests `strict_offline_allows_only_lan`, `default_allows_allowlist_and_lan_only`, `ipv4_mapped_ipv6_is_classified_by_its_ipv4`; `ghi-net` 49 tests pass | Verified |
| Strict offline refuses before any socket (downloads, update fetches, cloud) | `fetch::tests::strict_offline_is_refused_before_any_socket`, `strict_offline_reports_nothing_and_opens_nothing`; `CloudGrant::mint` refuses (`cloud::tests::mint_refuses_policy_and_gates`); CLI: `ghi models fetch qwen3-4b --strict-offline` exits 1 `not allowed: strict offline is on` | Verified |
| Resolver: public addresses only, no DNS rebinding, loopback never; no proxy from the environment; no redirects for cloud; allowlisted redirects only for downloads | `cloud::tests::resolver_filter_keeps_only_public_addresses`, `hostname_resolving_to_loopback_is_blocked_before_connecting`, `config_ignores_the_environment_and_refuses_redirects`, `redirects_are_refused_and_not_followed`; `fetch::tests::off_list_redirect_is_refused`, `redirect_loop_stops` | Verified |
| Only `ghi-net` opens connections | `tools/scripts/check-net-egress.sh`: `net egress: ok`; CI `ci.yml:29`; `deny.toml` bans HTTP crates elsewhere | Verified |
| A whole meeting works with the kernel denying the network | Ran: `sandbox-exec -p '(version 1)(allow default)(deny network*)' ghi session --speed 0 --process --replay <vi clip>` with the nemo `ghi` and the LLM worker: live transcript, final pass and both notes jobs `done`, exit 0 (the repo test `ghi-cli/tests/session.rs::a_whole_meeting_runs_with_the_network_denied` does the same) | Verified |
| Zero sockets observed | `tools/release/net-audit.sh --strict --cli` (record-only) and with engines (`ghi` + `ghi-llm-worker` seen): `sockets seen: 0`, `RESULT: PASS`; `--selftest` proves the sampler sees sockets. The real app under Little Snitch is the owner's step (`network-audit.md`) | Verified for CLI, owner for app |

## 5. Audio protocol tokens

| Check | Evidence | Result |
|---|---|---|
| The URL carries only a random 128-bit token; nothing from it becomes a path or gid; unknown/expired = 404 | `audio_protocol.rs:68-79` (OsRng, 16 bytes), `:519-524` (32 hex chars or 404); `only_tokens_reach_the_store` | Verified |
| Sample tokens are one span <= 30 s, valid 10 min; play token = latest only; revoked on discard/delete | `:36-37` (`TTL`, `MAX_SPAN_MS`), `:218-226`; tests `tokens_expire_and_are_revoked_per_meeting`, `play_tokens_answer_in_chunks_and_only_the_latest_works` | Verified |
| Audio is decrypted and decoded in memory, never written to disk; responses `no-store`, Range-capable, at most 1 MiB each | module doc + `MAX_CHUNK`; `ranges`, `the_whole_meeting_plays_as_one_wav_with_holes_as_silence`, `wav_header_is_16k_mono_pcm`; `cargo test -p ghi-desktop --lib -- audio_protocol`: 7 passed | Verified |
| Only the main window can issue tokens | `capabilities/default.json` lists `allow-issue-audio-sample` and `allow-issue-audio-play`; `detect`, `mini`, `popover` do not | Verified |
| Webview can only play via the scheme | CSP `media-src ghi-audio: http://ghi-audio.localhost` (nothing else) | Verified |

## 6. Redaction and CloudGrant

| Check | Evidence | Result |
|---|---|---|
| A grant binds scheme, host, port, path+query and SHA-256 of the body; `send` recomputes before opening a socket; mismatch opens no socket and keeps attempts | `ghi-net/src/cloud.rs:192-300`; tests `check_binds_host_port_path_and_body`, `mismatch_opens_no_socket_and_keeps_attempts`, `ghi-llm cloud::tests::a_grant_for_other_bytes_is_denied_without_a_connection` | Verified |
| https only, 10 min TTL, 3 attempts, only 429/5xx/transport retried with identical bytes, no redirects, errors carry no headers or bodies | `cloud.rs:41-44`; `expired_grant_is_refused`, `retries_only_429_and_5xx_with_identical_bytes_up_to_three_attempts`, `a_4xx_answer_spends_the_grant`, `secret::tests::secrets_never_print`, `ghi-llm keys_are_masked_in_provider_text` | Verified |
| Cloud sends transcript text only: preview is the redacted request, holds no audio or notes; its SHA-256 equals the body's | `ghi-core/src/cloud.rs` test `the_preview_is_the_redacted_request_and_holds_no_audio_or_notes` (:386), `ghi-llm preview::tests::preview_shows_the_exact_bytes_and_their_hash`; the app sends the held plan, not a rebuilt request (`cloud_cmd.rs` "Sends the previewed request, exactly") | Verified |
| Per-meeting opt-in and lock; transcript edit after preview invalidates it; strict offline refuses | `ghi-core/src/cloud.rs:160-170,244-258`; tests `a_locked_or_sensitive_meeting_is_refused`, `strict_offline_refuses_the_send_and_nothing_is_logged` | Verified |
| Redaction | `ghi-llm/src/redact.rs`; unit and property tests (`formatted_pii_never_survives`, `redact_then_restore_round_trips`, `plain_words_are_untouched`, `redact_never_panics`) all pass: 43 tests of `ghi-llm` redact/preview/cloud pass | Verified (F6) |
| API keys live in the Keychain, never in files or logs | `core.rs` `secrets()` (`KeychainSecrets`; debug file only under `debug_assertions`); smoke-check S-111 greps Application Support for the key | Partly (owner: S-111 on the signed build) |

## 7. Model hash checks

| Check | Evidence | Result |
|---|---|---|
| Each pinned file has revision, size and SHA-256; mirrors are https | `ghi-models/registry.toml`; `tests::registry_is_valid_and_pinned`, `mirrors_are_https_when_present` | Verified |
| Download: only allowlisted hosts, `.part` renamed into place only after size and SHA-256 match; tampered or half file is never the destination | `ghi-net/src/fetch.rs:9-14`; `bad_hash_leaves_nothing_in_place`, `same_size_file_with_a_bad_hash_is_downloaded_again`, `truncated_stream_keeps_part_and_overrun_drops_it` | Verified |
| Offline import only accepts a registry hash, copies, re-hashes, renames | `verify.rs:109-140`; `import_known_installs_and_unknown_is_refused`. Ran `ghi models import` with a same-size zero file: `bad_input: SHA-256 ... is not a known model; refused`; with the real file from `tools/release/offline-models.sh`: installed | Verified |
| Verified before native code parses it (every load path) | `verify_for_load` called at `ghi-llm/src/local.rs:87`, `desktop core.rs:146` (`checked_model`, used by `engines()` and the LLM), `ghi-cli session.rs:105` (not for explicit dev paths, F5); a damaged file marks the model `DAMAGED` for re-download (`core.rs:134-155`) | Verified (F5) |
| Mirror and offline package publish only registry-exact files | `tools/release/mirror-models.sh` and `offline-models.sh` verify size+SHA-256 first; tested with a tampered file: `BAD-SHA ... not publishing`, exit 1 | Verified |

## 8. Crash-report scrubbing (TODO)

| Check | Status |
|---|---|
| Whitelist scrubber tested with a planted key and Vietnamese text; reports local only; no send button; worker exit records | `crates/ghi-diag`: scrubber unit tests (first line, ≤300 chars, `$HOME`, long quotes, hex ≥32, non-ASCII words → `<text>`); `tests/panic_report.rs` plants a 64-hex key and a Vietnamese sentence in a panic and asserts neither reaches the report; `tests/no_content_in_panics.rs` fails if a `panic!`/`expect`/`unreachable!` in ghi-store or ghi-core interpolates text/segment/body/title/key/name identifiers. Reports and `ghira.log` stay in `<app data>/diagnostics`; no send path exists. F7: `NetPolicy::permits` logs the host only (no path or query), local file. Owner: force a crash in the signed app and read the report and the copied `.ips` | Verified |

## 9. Updater (TODO)

| Check | Status |
|---|---|
| Minisign-signed manifest with sequence, expiry, `pulled`, `min_supported`; archive SHA-256; Team-ID check of the new bundle; atomic swap; no downgrade; deferred while recording; toggle off = no traffic; fetch only through `ghi-net` `fetch_small` / `UPDATE_HOSTS` | `crates/ghi-update` tests: `a_signed_manifest_is_read_and_anything_else_refused` (one flipped byte, no key, garbage signature), `only_newer_versions_are_offered_and_the_kill_switch_warns`, `replays_expiry_channels_and_unsupported_versions`, `a_rejected_bundle_leaves_the_installed_app_alone`, `swaps_in_a_checked_bundle_and_keeps_the_old_one_until_cleanup`, `finds_the_bundle_and_refuses_translocated_copies`; `ghi-net` `small_documents_come_from_update_hosts_only` (strict offline: zero requests; off-list host or redirect refused; size cap). Archive SHA-256/size checked by `fetch`. Desktop: checks skipped while recording/jobs run and under strict offline (`update_cmd.rs`); unpack + `codesign` Team-ID check happen before jobs stop and before the swap. Inert until `FEED_URL` + `PUBLIC_KEYS` are set. Owner: alpha.1 → alpha.2 on a real /Applications install, and `net-audit.sh` with updates off | Verified (tests) |

## Owner-only checks on the signed build

1. **Key binding (F2)**: after first launch of the notarized app, `security find-generic-password -s com.nhtera.ghira -g` from Terminal must prompt or be denied (the item's ACL trusts the signed Ghira); in the data-protection case it is not found at all. Record which.
2. **Keychain prompt count** on first launch and after an update (same Team ID, same designated requirement): at most one prompt, none after "Always Allow".
3. **Hardened runtime and entitlements**: `codesign -d --entitlements - Ghira.app` shows only `device.audio-input`; every nested binary signed (`smoke-checklist.md` S-00c).
4. **Network**: Little Snitch strict-offline capture with the signed app, WebKit networking process included (`network-audit.md`, layers A and B).
5. **Proxy capture** of default mode (`network-audit.md`, layer C).
6. **Backups**: `tmutil isexcluded ~/Library/Application\ Support/com.nhtera.ghira` reports `[Excluded]` on the signed app.

## Re-running this verification

```sh
cargo test -p ghi-store --no-fail-fast -p ghi-net -p ghi-models
cargo test -p ghi-llm --lib -- redact preview cloud
cargo test -p ghi-core --lib -- cloud
cargo test -p ghi-desktop --lib -- audio_protocol navigation
(cd apps/desktop && pnpm exec vitest run tests/unit/csp.test.ts \
  && pnpm exec playwright test csp.spec.ts --project=chromium --project=webkit)
./tools/scripts/check-net-egress.sh
./tools/scripts/check-no-dev-key.sh target/release/ghi target/release/ghi-desktop
tools/release/net-audit.sh --selftest && tools/release/net-audit.sh --strict --cli
tools/release/crash-safety.sh -n 3
```
