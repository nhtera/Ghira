# Ghira website

The landing page and the docs at https://ghira.app. TanStack Start
(Vite 8) with `fumadocs-core` + `fumadocs-mdx` for the docs, Ghira's own
design tokens and fonts, built with the Cloudflare Vite plugin and the `cf`
CLI. Every page is prerendered to HTML; a tiny Worker answers only the
requests no file matches (the 404 page) and cannot render a page.

## Run it

Node 22.18 or later. The site is a standalone npm project: it is **not**
part of the pnpm workspace (see Isolation).

```sh
npm --prefix apps/website ci --ignore-scripts
npm --prefix apps/website run dev         # http://localhost:3100
npm --prefix apps/website run build:site  # sync + cf build + finalize; output in apps/website/.cloudflare/output/v0/
npm --prefix apps/website run preview     # the built site, served by the Workers runtime
npm --prefix apps/website test            # unit tests
npm --prefix apps/website run lint
npm --prefix apps/website run typecheck
npm --prefix apps/website run check-links # on the build output
```

## Where the content comes from

- **Docs:** `docs/README.md` is the nav. Each `## Section` heading is
  followed by a table of pages (`| [file.md](file.md) | Contents |`), in
  sidebar order. Only table rows under a `##` heading are published; a file
  that is not listed is not published (default-deny). Besides `docs/`, four
  root documents may be listed as `../PRIVACY.md`, `../SECURITY.md`,
  `../CONTRIBUTING.md`, `../TRADEMARKS.md`. `docs/release/` (checklists and
  audits) is never published: a row pointing there fails the build.
- `npm run sync` (`scripts/sync-docs.mjs`) copies the published docs into
  `content/` (generated, git-ignored) with frontmatter (title from the H1,
  description from the nav row, `source` for the Edit link, `lastUpdated`
  from git unless `GHIRA_SITE_NO_DATES=1`), and writes
  `content/generated/{nav,quickstart,inputs}.json`.
- **Install commands** have one source: the single `sh` block under
  README.md `## Quick start (from source, macOS)`. The landing page shows it,
  and a `<!-- quickstart -->` line in a doc is replaced with it.
- **Links** in docs are rewritten at build time (`src/lib/remark-ghira-links.ts`):
  a published doc becomes its site URL, any other repository file its GitHub
  page; only `https:` and `mailto:` schemes; a missing target, an image or
  another scheme fails the build; raw HTML is dropped.
- **Site links to docs** go through the published nav (`src/lib/site-links.ts`):
  a page that is not published yet links to `/docs`.
- **Look:** Ghira's design tokens (`packages/ui/src/tokens/tokens.css`) are
  imported by relative path, as they are, and processed by Tailwind (the file
  is Tailwind source). Fonts come from the same `@fontsource*` packages as the
  app; `src/styles/fonts.css` has the same faces as `packages/ui/src/fonts.css`
  (a test keeps them equal). Site copy lives in `src/content/*.ts`.

## Screenshots

The landing page images come from the apps' own e2e mocks
(`apps/desktop/tests/e2e/marketing.spec.ts`, `apps/mobile/tests/e2e/marketing.spec.ts`).
Recapture on macOS when the app UI they show changes; never in CI (the
specs skip unless `GHI_MARKETING=1`).

```sh
pnpm install
pnpm --filter @ghi/desktop build && pnpm --filter @ghi/ui gallery:build
pnpm --filter @ghi/mobile build
# once: npx playwright install webkit   (from apps/desktop and apps/mobile)
GHI_MARKETING=1 pnpm --filter @ghi/desktop exec playwright test marketing --project=webkit
GHI_MARKETING=1 pnpm --filter @ghi/mobile exec playwright test marketing
npm --prefix apps/website run screens
```

The specs write 8 PNGs (`desk-live`, `desk-notes`, `phone-live`,
`phone-meetings`, each light and dark) to `apps/website/.screens/`
(gitignored; override with `GHI_MARKETING_OUT`). They are deterministic
(seeded `Math.random`, Playwright's fixed clock, reduced motion). `screens`
turns them into hashed WebP in `public/screens/` and rewrites
`src/content/screens.json`. A WebP is replaced only when the image visibly
changed, so recapturing an unchanged UI changes no files. Commit the WebP and
the manifest, never the PNGs. The total must stay under 2.5 MiB.

## Isolation

`apps/website` has its own `package-lock.json` and is installed with
`npm ci --ignore-scripts`; `pnpm-workspace.yaml`, `pnpm-lock.yaml` and the
root scripts do not know it. So the beta Cloudflare tooling and its install
scripts never reach the workspace installs, including the release job that
holds the Apple signing secrets, and root `pnpm -r` stays fast.
`npm run check:install-scripts` fails on any package with an install script
that is not on its allowlist; `.github/workflows/site.yml` runs it and
`npm audit --audit-level=high` for both lockfiles (the site and `deploy/`).

## Decisions from the spike

Recorded 2026-10-09 with `@tanstack/react-start` 1.168.60,
`@cloudflare/vite-plugin` 2.0.0-beta.sha-52b0dc0e9, `cf` 1.0.0-beta.12,
`fumadocs-core` 16.16.2, `fumadocs-mdx` 15.4.6, `vite` 8.3.2, TypeScript 5.9.3.
Ported from Sonde's site (same owner, Apache-2.0), which runs this pipeline
in production.

- **(a) Prerender: passed.** `cf build` prerenders `/`, `/docs`, every docs
  page, `/404.html` and the JSON/text routes listed in `vite.config.ts`
  `pages` (crawl + `failOnError`).
- **(b) Output path:** `cf build` writes the Cloudflare Build Output to
  `.cloudflare/output/v0/` (`workers/default/assets/` = the static site,
  `workers/default/bundle/` = the Worker). `scripts/paths.mjs` names them.
- **(c) Preview: passed.** `npm run preview` (workerd): assets served,
  `/docs/x/` → `/docs/x` (307), an unknown path → the 404 page, status 404,
  with the security headers and its CSP.
- **(d) The deployed Worker cannot render: passed.** The build's Worker
  (`src/server.ts`) renders only for prerender requests carrying a per-build
  token (`GHIRA_PRERENDER_TOKEN`). It is never deployed: `npm run finalize`
  rebuilds the bundle from `src/worker/production.ts`, which imports nothing
  from TanStack Start, and `scripts/build-worker.mjs` fails if the bundle has
  anything but `index.js`, any TanStack/Fumadocs code or the token header.
  The deployed Worker is about 3 KB.
- **(e)** `typecheck`, `lint` and `test` pass.
- **(f)** Nothing outside `apps/website` changed behaviour: root
  `pnpm install` / `pnpm build` are unchanged (the site is not a workspace
  package).
- **(g)** `./tools/scripts/check-spdx.sh` passes.
- **(h)** `npm ci --ignore-scripts` + `check:install-scripts` pass: no
  package needs an install script (esbuild and workerd ship their binaries as
  optional dependencies).
- **Tokens without a layer.** `tokens.css` contains `@custom-variant`, which
  Tailwind refuses inside `@layer`, so it is imported unlayered; it only
  defines custom properties the site never redefines. The site's own CSS is
  the `components` layer. Tailwind's preflight is not imported: the site's
  base is the prototype's.
- **Font preloads** use `?url` imports of the same files the `@font-face`
  rules use (Sonde's approach): Vite emits one asset per file, so the
  preload and the face share a URL and nothing downloads twice.
- **Search index budget on the gzip size.** The full-text index is about
  780 KB of JSON but 180 KB gzipped (110 KB with Cloudflare's brotli), and it
  loads only on the first search. `src/lib/search-index.ts` budgets 250 KB
  gzipped and falls back to headings + first paragraphs, then headings only,
  if the docs outgrow it (the build logs a warning).
- **Markdown, not MDX.** fumadocs-mdx compiles `.md` as Markdown: `{…}` is
  text, raw HTML is dropped (`test/fixtures/hostile.md`).

## Deploy

`.github/workflows/site.yml`, with the `cf` CLI (beta, pinned in
`package.json` and `deploy/package.json`; a test keeps them equal):

- `changes`: runs the site jobs only when a pull request, or a push to
  `main`, touches `SITE_PATHS` (the site, `docs/`, the published root
  documents, the token and font files, the locale files and the sample
  meeting). `scripts/ci-paths.mjs covers` fails a build that reads a
  repository file the filter does not list.
- `site-build` (no secrets): installs without scripts, checks install scripts,
  licences and `npm audit` for both lockfiles, then lint, typecheck, tests,
  `cf build`, `finalize` (static 404, `_headers`, the 404-only Worker),
  `check-links`, the browser suites, "the build changed no file", and
  `cf deploy --prebuilt --dry-run` with the production pin. Uploads the output
  on `main` when deploys are on.
- `site-gate`: always runs; the one check to require in branch protection.
- `site-deploy` (`main` only, environment `site`, **only when the repository
  variable `SITE_DEPLOY` is `1`**): installs only `deploy/` (the pinned `cf`),
  deploys the artifact with `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`
  on that one step, then runs `deploy/smoke.sh`: against https://ghira.app
  when `SITE_LIVE` is `1`, else against `SITE_WORKERS_URL` if set.
- `DO_NOT_TRACK=1` turns off the `cf` CLI's telemetry in CI.

### One-time setup (owner)

1. Cloudflare API token, account-owned: **Workers Scripts: Edit** (add
   **Account Settings: Read** only if `cf` asks for it), expiring after one
   year; rotate yearly. The same Cloudflare account hosts `sonde-site`
   (accepted 2026-10-09): a Workers token from either repository could
   overwrite the other's Worker, so keep both tokens scoped, short-lived and
   rotated.
2. GitHub environment `site`: deployment branch `main`, a required reviewer,
   secrets `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`. Branch
   protection: require `site-gate`.
3. Set `SITE_DEPLOY=1`. The first deploy goes to
   `ghira-website.<account>.workers.dev`; set `SITE_WORKERS_URL` to that URL
   so the smoke test runs against it.

### Cutover to ghira.app (owner present; log each step here, no secret values)

1. Zone settings for `ghira.app`, **off**: Email Address Obfuscation, Rocket
   Loader, Bot Fight Mode / JavaScript detections, Automatic Signed
   Exchanges, Web Analytics / RUM, managed robots.txt, AI-crawler blocking.
   **On**: Always Use HTTPS, minimum TLS 1.2. No zone HSTS (the site's
   `_headers` sends it).
2. `cf dns records list -z ghira.app`: confirm the 4 parking records
   (apex A ×2, `www` CNAME, `*` CNAME), then delete them only with the
   owner's OK.
3. Attach the apex Custom Domain `ghira.app` to the `ghira-website` Worker in
   the dashboard (the config has no `domains`, so the token needs no zone
   access). Redeploy once (workflow_dispatch) and confirm the domain is still
   attached.
4. `www`: a proxied `A www 192.0.2.0` record and one Single Redirect rule:
   `http.host eq "www.ghira.app"` → `concat("https://ghira.app", http.request.uri.path)`,
   301, keep the query string.
5. Email Routing: enable it, verify the owner's inbox, add routes for
   `security@` and `conduct@`, set catch-all to drop; add
   `_dmarc TXT "v=DMARC1; p=reject; adkim=s; aspf=s"` and CAA records for the
   CAs Cloudflare uses; send a test mail to each address. Then add
   `security@ghira.app` to SECURITY.md (GitHub private reporting stays first)
   and `conduct@ghira.app` to CODE_OF_CONDUCT.md.
6. When every page in `docs/README.md` is published and SECURITY.md names
   the address, set `SITE_LIVE=1` and re-run the deploy: the smoke test then
   also checks the www redirect, every live page and the mail address. Run
   the landing privacy suite once against production:
   `BASE_URL=https://ghira.app npm run test:browser -- --test-name-pattern=privacy`.

### Cutover log

2026-10-09, done with the owner's go-ahead (no secret values here):

- First deploys from a Mac with the pinned `cf` (OAuth login, not a stored
  token): version `cdbc1fa2` on `ghira-website.tienn.workers.dev`, smoke
  passed; then `cd8de996` (same build) to check that the hand-attached domain
  survives a deploy: it does.
- Zone `ghira.app` (Free plan): Email Address Obfuscation **off** (was on),
  Web Analytics RUM auto-install **off** (was on), Always Use HTTPS **on**
  (was off), minimum TLS **1.2** (was 1.0). Already off: Rocket Loader, Bot
  Fight Mode, JavaScript detections, managed robots.txt, AI-crawler blocking.
- Deleted the 4 Porkbun parking records (apex A 207.207.210.107 and
  207.207.210.229, `www` and `*` CNAME pixie.porkbun.com, all proxied).
- Apex Custom Domain `ghira.app` → `ghira-website` (certificate: Let's
  Encrypt, issued automatically).
- `www`: proxied `A www 192.0.2.0` and the Single Redirect rule
  (`http.host eq "www.ghira.app"`, 301, keeps path and query).
- Checks on https://ghira.app: `deploy/smoke.sh https://ghira.app 1` passes
  except the `security@ghira.app` mail address (Email Routing not set up
  yet); browser suites against production (`BASE_URL=https://ghira.app`):
  privacy (same-origin only, no cookies), CSP (6 pages, search), docs (17)
  all pass; `http://` and `www` redirect with 301.
- Rollback tried: `cd8de996` → `cdbc1fa2` → `cd8de996`, the site answered
  200 throughout.
- GitHub environment `site` created (required reviewer, branch `main`).
  Still to do by the owner: the account API token and the
  `CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_ACCOUNT_ID` secrets, then
  `SITE_DEPLOY=1`; branch protection requiring `site-gate`; Email Routing
  (step 5), then `SITE_LIVE=1`.

## Rollback

With a Cloudflare login (`npx cf auth login`) or `CLOUDFLARE_API_TOKEN` and
`CLOUDFLARE_ACCOUNT_ID` in the environment:

```sh
cd apps/website/deploy && npm ci --ignore-scripts
npx cf workers deployments list --worker ghira-website      # the active version
npx cf workers versions list --worker-id ghira-website      # pick the previous version id
npx cf workers deployments create --worker ghira-website --strategy percentage \
  --versions '[{"version_id":"<previous-id>","percentage":100}]'
```

With Wrangler: `npx wrangler rollback --name ghira-website`. Roll forward by
deploying the newer version id the same way, or by re-running the `site`
workflow on `main`. Try a rollback once after the first good deploy.

## Fallback: Wrangler

If a `cf` beta breaks the build or deploy, use the GA path: replace
`@cloudflare/vite-plugin` with `1.62.5` and `cloudflare.config.ts` with
`wrangler.jsonc`:

```jsonc
{
  "name": "ghira-website",
  "compatibility_date": "2026-10-01",
  "compatibility_flags": ["nodejs_compat"],
  "main": "./src/server.ts",
  "assets": { "binding": "ASSETS", "html_handling": "drop-trailing-slash" }
}
```

Build with `vite build` (output in `dist/client` and `dist/server`; point
`scripts/paths.mjs` there), and deploy with a pinned `wrangler` in
`apps/website/deploy/` (`wrangler deploy`).

## Claim audit

Date: 2026-10-09 (Phase 8). Every product claim on the site needs a source in
the code or a reworded cell. Paths are relative to the repo root.

### Landing and FAQ

| Claim (where) | Source |
|---|---|
| Pre-release, Apple Silicon, macOS 14.2+ (hero status, get, FAQ, llms) | `apps/desktop/src-tauri/tauri.release.conf.json:7` (`minimumSystemVersion`), `tools/release/build-dmg.sh:12,98` (aarch64, `min_macos`), `docs/install.md:8` |
| English, Vietnamese, both in one sentence (hero, FAQ) | `docs/overview.md`; notes language switch `docs/notes.md:57-59` |
| No bot, mic + meeting app sound (listen, FAQ) | `native/macos/GhiAudioMac/Sources/GhiAudioMac/AggregateDevice.swift:4` (mic + process tap), `crates/ghi-audio/src/detect.rs:179` |
| Zoom, Meet, Teams, "anything that plays sound" (listen) | `detect.rs:62-63` (Zoom, Teams bundle ids); browsers prompt generically (`detect.rs:8-14`); no listed app in a call: whole system is captured (`detect.rs:179-180`) |
| Offers to record when a call starts (listen) | `apps/desktop/src-tauri/src/system.rs:8-10` poller; setting "Ask to record when a call starts" (`en.json:710`), opt-in per `docs/recording.md:19-24` |
| Room mode, Mac and iPhone (listen) | `en.json:28,405`; `docs/recording.md:8-11`; `apps/mobile` record screens |
| Speaker color + number, rename live (apps) | `packages/ui/src/components/speaker-chip/speaker-chip.tsx:3,37` |
| Notes: summary, decisions, actions, questions, citations (after) | `crates/ghi-llm` templates + citations; `docs/notes.md` |
| Notes written by a model on the Mac (after) | `crates/ghi-llm-worker`; registry `qwen3-4b` (`crates/ghi-models/registry.toml:29`) |
| iPhone in testing, Simulator only, hands to Mac over own network (hero, apps, features, get) | `CLAUDE.md` iOS section; `docs/iphone-sync.md` Status; `crates/ghi-sync` |
| Never sent: meetings; exceptions cloud AI + LAN sync (privacy) | `PRIVACY.md:9-11`; `crates/ghi-net` (`CloudGrant`, `lan`) |
| Model downloads Hugging Face only; update check off until signed releases (privacy) | `PRIVACY.md:11,19-30`; `crates/ghi-update/src/lib.rs:19,28` (`FEED_URL: None`) |
| Strict offline turns both off (privacy) | `en.json:924-925,1115,887`; `PRIVACY.md:11,30`; `docs/models.md:53` |
| No telemetry; crash reports local, you can send them (privacy) | `PRIVACY.md:12`; `crates/ghi-diag` |
| Fonts and icons bundled (privacy) | `PRIVACY.md:13`; `packages/ui` gen:fonts, CSP |
| Cloud AI per meeting, exact preview, redaction, no audio, no typed notes, own key (privacy) | `PRIVACY.md:41-47`; `crates/ghi-core/src/cloud.rs`; `docs/cloud-ai.md` |
| No account, no server (FAQ) | `PRIVACY.md:41`; `CLAUDE.md` invariants |
| Models "a few GB", download once, SHA-256 pinned (get, FAQ) | `registry.toml` sizes: 742+107+2497+639+28 MB, about 4 GB; `docs/models.md:36-41`; `PRIVACY.md:19-22` |
| Matching by meaning needs Balanced or Max (features) | `crates/ghi-models/src/tier.rs:152` (`embed_id: (tier != Tier::Light)`); `crates/ghi-core/src/ask_all.rs:13` (semantic skipped without the embedder) |
| Accents optional, Ask across meetings with sources (features) | `ghi-store` VN-folded FTS5; `crates/ghi-core/src/ask_all.rs` |
| Import Voice Memos, Plaud, Zoom per-participant, any file (features) | `crates/ghi-core/src/presets.rs:18`; `import::import_tracks` |
| Export Markdown, Word, text, subtitles, Obsidian, follow-up email (features) | `crates/ghi-core/src/export.rs:2-3`; `crates/ghi-core/src/email.rs` |
| Own key per meeting; delete destroys it (features) | `crates/ghi-store` per-meeting DEKs, crypto-shred delete; `docs/overview.md` |
| Calendar optional, attendee names (features) | `PRIVACY.md:52-58`; `crates/ghi-core/src/calendar.rs`; `docs/calendar.md` |
| Voice profile with consent; others later (features) | `PRIVACY.md:49-52`; `THIRD_PARTY_APPROVED` in `system.rs` (hard-off) |
| App lock: Touch ID or password; recording continues (features) | `crates/ghi-app/src/lock_cmd.rs:119-134` (`DeviceOwnerAuthentication`); `CLAUDE.md` P1 (recording uses `store_even_locked`) |
| Windows not shipped; code compiles (get, FAQ) | `.github/workflows/ci.yml:57` (windows-latest build); `docs/install.md:86` |
| Android: no app (get, FAQ) | no Android target in the repo |
| Build needs Rust, Node 22 + pnpm, CMake 3.26+, Xcode (get) | `docs/install.md:9-13` |
| Demo is a sample meeting from the test set (demo) | `apps/website/src/content/demo-data.ts` header |

### Comparison rows (landing.ts compare)

Read October 2026. "Typical" = public docs of well-known tools.

| Row | Sources checked | Result |
|---|---|---|
| Audio processed on their servers | tl;dv https://tldv.io/features/security-commitment/ ; Otter https://help.otter.ai/hc/en-us/articles/360048322493-Transcription-processing-time-FAQ | tl;dv: data processed in its GCP/Hetzner data centers, confirmed. Otter (read in a browser 2026-10-09): processing starts after you finish recording or upload the audio, on Otter's processing system. Confirmed |
| Something joins the call | Fireflies https://guide.fireflies.ai/articles/6388921822-how-to-add-fireflies-to-a-meeting-as-a-participant ; Fathom https://help.fathom.video/en/articles/13114369 | Both confirm a notetaker bot joins ("Fathom Notetaker joining the call"). "Often a bot" fits |
| Account required | Fathom https://help.fathom.video/en/articles/276608 ; Otter terms https://otter.ai/terms-of-service | Fathom: "create your account". Otter 3.1: "you must register for an account" for most features. Confirmed |
| Works with no internet | Otter processing times (above); Fireflies https://guide.fireflies.ai/articles/1360888790-how-to-upload-unprocessed-files-in-the-fireflies-mobile-app | Fireflies: offline recordings wait "until you're back online" to upload and transcribe. Confirmed. Otter: processing starts after upload. Confirmed |
| Vietnamese / mixed | Otter https://help.otter.ai/hc/en-us/articles/360047247414-Supported-languages ; Fireflies https://guide.fireflies.ai/articles/2585231364-transcribe-fireflies-meetings-in-multiple-languages-with-multi-language-mode-beta | Fireflies: 60+ languages incl. Vietnamese, word-level switching. Otter (read in a browser 2026-10-09): English, Spanish, French, German, Japanese, Chinese only. "Varies by tool" holds |
| Closed source | Otter terms https://otter.ai/terms-of-service ; Fireflies https://fireflies.ai/terms-of-service | Both: proprietary, no reverse engineering (Otter 6 and 11(f)(ii); Fireflies 6(a), 9, 11(a)). Neither says "closed source" literally |

### Docs and PRIVACY (checked by page)

- overview, install, getting-started: `tauri.release.conf.json`, `build-dmg.sh`, `ci.yml`, `ghi-update` `FEED_URL None`.
- models: `crates/ghi-models/registry.toml` (sizes), `tier.rs` (Light < 12 GiB, Balanced < 28 GiB), `ghi-net` allowlist.
- recording, shortcuts: `crates/ghi-audio/src/detect.rs`, `apps/desktop/src-tauri/src/menu.rs:44,69`; bold labels match `packages/i18n/locales/en.json`.
- iphone-sync: `crates/ghi-sync`, `packages/i18n/locales/mobile/{sync,settings}.en.json` (labels), `PRIVACY.md`.
- settings, speakers, notes, search, import, export, calendar, cloud-ai, cli: UI labels spot-checked against `en.json` (about 30 bold labels, all found; "Search or Jump To…" is the macOS menu text in `menu.rs`), behavior against the `ghi-core` modules named in `CLAUDE.md`.
- PRIVACY.md: `crates/ghi-net`, `crates/ghi-update`, `tools/release/net-audit.sh`, `crates/ghi-diag`; no change needed.
- Release notes 0.1.0-alpha.1: historical; a note was added under the H1 (EN and VI).

### Changed

- Landing "Ghira notices when a call starts and offers to record." became "Turn on detection and Ghira offers to record when a call starts." The setting is off until you enable it (`docs/recording.md:19-24`, `system.rs`).
- FAQ "never leaves it" became "never goes to the internet": paired-phone sync sends audio to the Mac over the local network (`PRIVACY.md:9`).
- Compare row "Works with no internet": the Fireflies troubleshooting page only said to use a stable connection; swapped for Fireflies' offline-recordings article, which says transcription waits for a connection.
- `docs/release-notes/0-1-0-alpha-1.md`: added a note (EN and VI) that plans changed (no update check yet; lock, calendar, voice profiles, sync, iPhone app now built) with a link to PRIVACY.md. The body is untouched.

### Owner review

- Otter help pages (processing times, languages) block plain fetchers (Cloudflare); both were read in a headless browser on 2026-10-09 and support their rows.
- The "Closed source" cell rests on proprietary-license terms, not the words "closed source".
