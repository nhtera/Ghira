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
- **Markdown, not MDX.** fumadocs-mdx compiles `.md` as Markdown: `{…}` is
  text, raw HTML is dropped (`test/fixtures/hostile.md`).

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
