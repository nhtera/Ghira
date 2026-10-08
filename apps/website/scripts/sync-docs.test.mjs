// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { parse } from "yaml";
import { expandQuickstart, frontmatter, parseQuickstart, splitTitle, sync } from "./sync-docs.mjs";

test("the title is the first H1 after HTML comments", () => {
  const { title, body } = splitTitle("<!-- Generated.\n Do not edit. -->\n\n# The `ghi` command\n\nRun it.\n");
  assert.equal(title, "The ghi command");
  assert.equal(body, "Run it.\n");
  assert.throws(() => splitTitle("Intro\n\n# Late title"), /first line/);
});

test("YAML-hostile titles and descriptions round-trip", () => {
  const data = { title: 'Notes: "and" citations', description: "a: b # c — `x` 'y' {z} [w] *v &u !t |s >r", source: "docs/x.md" };
  const text = frontmatter(data);
  assert.deepEqual(parse(text.replace(/^---\n|---\n\n$/g, "")), data);
});

test("the quick start comes from README.md, one sh block", () => {
  const readme = readFileSync(new URL("../../../README.md", import.meta.url), "utf8");
  const { commands } = parseQuickstart(readme);
  assert.match(commands, /^git clone https:\/\/github\.com\/nhtera\/Ghira\.git && cd Ghira\n/);
  assert.ok(!commands.includes("```"));
  assert.throws(() => parseQuickstart("# x\n## Other\n"), /no "## Quick start/);
  assert.throws(() => parseQuickstart("# x\n\n## Quick start (from source, macOS)\n\ntext\n\n## Next\n"), /exactly one/);
  assert.throws(() => parseQuickstart("# x\n\n## Quick start (from source, macOS)\n\n```sh\na\n```\n\n```sh\nb\n```\n"), /exactly one/);
  assert.equal(expandQuickstart("Before\n<!-- quickstart -->\nAfter", { commands: "a\nb" }), "Before\n```sh\na\nb\n```\nAfter");
});

/** A minimal repository with the docs layout the nav expects. */
function fixtureRepo({ shallow = false } = {}) {
  const repo = mkdtempSync(join(tmpdir(), "ghira-sync-"));
  const w = (rel, text) => {
    mkdirSync(join(repo, rel, ".."), { recursive: true });
    writeFileSync(join(repo, rel), text);
  };
  w(
    "docs/README.md",
    "# Docs\n\n## Get started\n\n| Page | Contents |\n|---|---|\n| [install.md](install.md) | Build from source |\n\n## Project\n\n| Page | Contents |\n|---|---|\n| [Privacy](../PRIVACY.md) | What may leave your device |\n| [release-notes/0-1-0-alpha-1.md](release-notes/0-1-0-alpha-1.md) | First alpha |\n\n## Repository files (not published)\n\n- [Smoke](release/smoke.md)\n",
  );
  w("docs/install.md", "# Install from source\n\nRun this:\n\n<!-- quickstart -->\n\nSee [privacy](../PRIVACY.md).\n");
  w("docs/release-notes/0-1-0-alpha-1.md", "# Ghira 0.1.0-alpha.1\n\nNotes.\n");
  w("docs/release/smoke.md", "# Smoke\n");
  w("PRIVACY.md", "# Privacy\n\nNothing leaves.\n");
  w("README.md", "# Ghira\n\n## Quick start (from source, macOS)\n\nNeeds Rust.\n\n```sh\ngit clone x && cd y\npnpm install\n```\n\nMore.\n\n## Checks\n");
  const git = (...args) => execFileSync("git", args, { cwd: repo, env: { ...process.env, GIT_CONFIG_GLOBAL: "/dev/null", GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@t", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@t" } });
  git("init", "-q", "-b", "main");
  git("add", "-A");
  git("commit", "-qm", "init");
  if (!shallow) return repo;
  const clone = mkdtempSync(join(tmpdir(), "ghira-sync-shallow-"));
  execFileSync("git", ["clone", "-q", "--depth", "1", `file://${repo}`, clone]);
  rmSync(repo, { recursive: true, force: true });
  return clone;
}

test("sync writes docs with frontmatter, meta.json, nav, quick start and inputs", () => {
  const repo = fixtureRepo();
  const out = join(repo, "out");
  const logs = [];
  const { unpublished } = sync({ repo, out, log: (m) => logs.push(m) });
  const install = readFileSync(join(out, "docs/install.md"), "utf8");
  assert.match(install, /^---\ntitle: Install from source\ndescription: Build from source\nsource: docs\/install\.md\nlastUpdated: \d{4}-\d{2}-\d{2}\n---\n\nRun this:\n\n```sh\ngit clone x && cd y\npnpm install\n```\n/);
  assert.match(readFileSync(join(out, "docs/privacy.md"), "utf8"), /source: PRIVACY\.md/);
  assert.match(readFileSync(join(out, "docs/release-notes/0-1-0-alpha-1.md"), "utf8"), /title: Ghira 0\.1\.0-alpha\.1/);
  assert.deepEqual(JSON.parse(readFileSync(join(out, "docs/meta.json"), "utf8")).pages, ["index", "---Get started---", "install", "---Project---", "privacy", "release-notes/0-1-0-alpha-1"]);
  const nav = JSON.parse(readFileSync(join(out, "generated/nav.json"), "utf8"));
  assert.deepEqual(nav.sections.map((s) => [s.title, s.pages.map((p) => p.slug)]), [["Get started", ["install"]], ["Project", ["privacy", "release-notes/0-1-0-alpha-1"]]]);
  assert.equal(nav.sections[0].pages[0].title, "Install from source");
  // The nav name is the row's label when it has one; the page keeps its H1.
  assert.equal(nav.sections[1].pages[0].title, "Privacy");
  assert.match(readFileSync(join(out, "docs/privacy.md"), "utf8"), /^---\ntitle: Privacy\n/);
  assert.deepEqual(JSON.parse(readFileSync(join(out, "generated/quickstart.json"), "utf8")), { commands: "git clone x && cd y\npnpm install" });
  assert.deepEqual(unpublished, ["docs/release/smoke.md"]);
  assert.ok(logs.some((l) => l.startsWith("unpublished docs: docs/release/smoke.md")));
  const inputs = JSON.parse(readFileSync(join(out, "generated/inputs.json"), "utf8"));
  for (const p of ["README.md", "docs/README.md", "docs/install.md", "PRIVACY.md"]) assert.ok(inputs.includes(p), p);
  // Idempotent: a second run gives the same files.
  sync({ repo, out, log: () => {} });
  assert.equal(readFileSync(join(out, "docs/install.md"), "utf8"), install);
  rmSync(repo, { recursive: true, force: true });
});

test("sync fails on a missing doc, a refused row, a missing H1, an image and a shallow clone", () => {
  const run = (repo, extra = {}) => sync({ repo, out: join(repo, "out"), dates: false, log: () => {}, ...extra });
  let repo = fixtureRepo();
  rmSync(join(repo, "docs/install.md"));
  assert.throws(() => run(repo), /docs\/install\.md: listed in the docs nav but missing/);
  rmSync(repo, { recursive: true, force: true });

  repo = fixtureRepo();
  writeFileSync(join(repo, "docs/README.md"), readFileSync(join(repo, "docs/README.md"), "utf8").replace("| [install.md](install.md) |", "| [release/smoke.md](release/smoke.md) |"));
  assert.throws(() => run(repo), /never published/);
  rmSync(repo, { recursive: true, force: true });

  repo = fixtureRepo();
  writeFileSync(join(repo, "docs/install.md"), "No heading here.\n");
  assert.throws(() => run(repo), /"# Title" heading/);
  rmSync(repo, { recursive: true, force: true });

  repo = fixtureRepo();
  writeFileSync(join(repo, "PRIVACY.md"), "# Privacy\n\n![x](x.png)\n");
  assert.throws(() => run(repo), /images are not supported/);
  rmSync(repo, { recursive: true, force: true });

  repo = fixtureRepo({ shallow: true });
  assert.throws(() => sync({ repo, out: join(repo, "out"), log: () => {} }), /shallow clone/);
  run(repo);
  assert.doesNotMatch(readFileSync(join(repo, "out/docs/install.md"), "utf8"), /lastUpdated/);
  rmSync(repo, { recursive: true, force: true });
});
