// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { test } from "node:test";
import { loadNav, parseNav, plainText, publishedSources } from "./docs-nav.ts";

const repo = new URL("../../../../", import.meta.url).pathname;

const FIXTURE = `# Ghira docs

<!-- comment -->

## Get started

| Page | Contents |
|---|---|
| [overview.md](overview.md) | What Ghira is, and what ships **today** |
| [install.md](install.md) | Build from source with \`pnpm\` |

## Reference

| Page | Contents |
|---|---|
| [Privacy](../PRIVACY.md) | What may leave your device |

## Project

| Page | Contents |
|---|---|
| [release-notes/0-1-0-alpha-1.md](release-notes/0-1-0-alpha-1.md) | The first alpha |

## Repository files (not published)

- [SECURITY](../SECURITY.md): reporting
- [Smoke checklist](release/smoke-checklist.md)
- [LICENSE](../LICENSE)

\`\`\`md
| [ignored.md](ignored.md) | inside a code fence |
\`\`\`
`;

test("sections and rows parse in order, with plain-text descriptions; bullets are ignored", () => {
  const nav = parseNav(FIXTURE);
  assert.deepEqual(nav.map((s) => s.title), ["Get started", "Reference", "Project"]);
  assert.deepEqual(nav[0].docs, [
    { source: "docs/overview.md", slug: "overview", description: "What Ghira is, and what ships today" },
    { source: "docs/install.md", slug: "install", description: "Build from source with pnpm" },
  ]);
  // A link text that is a label (not the file name) becomes the sidebar name.
  assert.deepEqual(nav[1].docs[0], { source: "PRIVACY.md", slug: "privacy", description: "What may leave your device", label: "Privacy" });
  assert.equal(nav[0].docs[0].label, undefined);
  assert.equal(nav[2].docs[0].slug, "release-notes/0-1-0-alpha-1");
  const published = publishedSources(nav);
  assert.ok(!published.has("SECURITY.md"), "a bullet link is not published");
  assert.ok(!published.has("docs/ignored.md"));
  assert.equal(published.get("docs/README.md"), "");
});

test("refused rows fail: docs/release, other root files, duplicates, rows before a section", () => {
  const nav = (row: string) => parseNav(`# Docs\n\n## S\n\n| Page | Contents |\n|---|---|\n${row}\n`);
  assert.throws(() => nav("| [release/smoke-checklist.md](release/smoke-checklist.md) | x |"), /never published/);
  assert.throws(() => nav("| [../LICENSE](../LICENSE) | x |"), /outside docs/);
  assert.throws(() => nav("| [../README.md](../README.md) | x |"), /outside docs/);
  assert.throws(() => nav("| [a.md](a.md) | x |\n| [a.md](a.md) | y |"), /listed twice/);
  assert.throws(() => parseNav("# Docs\n\n| [a.md](a.md) | x |\n"), /not under a "## Section"/);
  assert.throws(() => parseNav("# Docs\n\n## Only bullets\n\n- [a](a.md)\n"), /no table rows/);
});

test("the repository's docs/README.md parses and publishes nothing under docs/release", () => {
  const published = publishedSources(loadNav(repo));
  assert.ok(published.has("PRIVACY.md"));
  for (const source of published.keys()) assert.ok(!source.startsWith("docs/release/"), source);
});

test("markdown to plain text", () => {
  assert.equal(plainText("See [x](x.md) and `--jobs` **now**"), "See x and --jobs now");
});
