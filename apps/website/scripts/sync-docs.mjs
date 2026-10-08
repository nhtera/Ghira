// SPDX-License-Identifier: Apache-2.0

// Copies the published docs (docs/README.md is the nav) into
// apps/website/content/docs (generated, git-ignored) as Markdown with
// frontmatter, and writes the generated data the site reads from the
// repository:
//
//   content/docs/**.md, meta.json      the docs, H1 moved into frontmatter
//   content/generated/nav.json         sections → pages (slug, title, description)
//   content/generated/quickstart.json  the README "Quick start" commands
//   content/generated/inputs.json      every repository file this step read
//
// A `<!-- quickstart -->` line in a doc is replaced with the README Quick
// start block (one source for the install commands).
//
// Fails fast on a missing doc, a doc without an H1, a refused nav row, an
// image, or a shallow clone (dates). Runs no shell: git is called with
// execFileSync and fixed arguments.
//
//   node scripts/sync-docs.mjs   (GHIRA_SITE_NO_DATES=1 omits dates)

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, posix, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { stringify } from "yaml";
import { contentPathOfSlug, repoDir, SOURCE_PATH } from "../src/lib/doc-paths.ts";
import { loadNav, publishedSources } from "../src/lib/docs-nav.ts";

const here = dirname(fileURLToPath(import.meta.url));
export const SITE = resolve(here, "..");
export const REPO = resolve(SITE, "../..");

/** First `# ` heading, after any leading HTML comments and blank lines. */
export function splitTitle(md, file = "doc") {
  const lines = md.split("\n");
  let i = 0;
  let inComment = false;
  for (; i < lines.length; i++) {
    const line = lines[i].trim();
    if (inComment) {
      if (line.includes("-->")) inComment = false;
      continue;
    }
    if (line === "") continue;
    if (line.startsWith("<!--")) {
      inComment = !line.includes("-->");
      continue;
    }
    break;
  }
  const m = lines[i]?.match(/^#\s+(.+?)\s*#*\s*$/);
  if (!m) throw new Error(`${file}: the first line (after comments) must be the "# Title" heading`);
  const body = lines.slice(i + 1).join("\n").replace(/^\n+/, "");
  return { title: m[1].replace(/`([^`]*)`/g, "$1"), body };
}

/** YAML frontmatter, serialized (never string-templated). */
export function frontmatter(data) {
  return `---\n${stringify(data, { lineWidth: 0 })}---\n\n`;
}

const QUICKSTART_HEADING = "## Quick start (from source, macOS)";
export const QUICKSTART_MARKER = "<!-- quickstart -->";

/** The single ```sh block under README.md "## Quick start (from source, macOS)". */
export function parseQuickstart(readme) {
  const start = readme.indexOf(`\n${QUICKSTART_HEADING}\n`);
  if (start < 0) throw new Error(`README.md: no "${QUICKSTART_HEADING}" section`);
  const rest = readme.slice(start + QUICKSTART_HEADING.length + 2);
  const section = rest.split(/\n## /)[0];
  const blocks = [...section.matchAll(/^```sh\n([\s\S]*?)^```$/gm)];
  if (blocks.length !== 1) throw new Error(`README.md ${QUICKSTART_HEADING}: expected exactly one \`\`\`sh block, found ${blocks.length}`);
  const commands = blocks[0][1].replace(/\n+$/, "");
  if (!commands.trim()) throw new Error(`README.md ${QUICKSTART_HEADING}: the sh block is empty`);
  return { commands };
}

/** Replace a `<!-- quickstart -->` line with the Quick start block. */
export function expandQuickstart(body, quickstart) {
  return body
    .split("\n")
    .map((line) => (line.trim() === QUICKSTART_MARKER ? `\`\`\`sh\n${quickstart.commands}\n\`\`\`` : line))
    .join("\n");
}

function lastUpdated(repo, repoPath) {
  const out = execFileSync("git", ["log", "-1", "--format=%cs", "--", repoPath], { cwd: repo, encoding: "utf8" }).trim();
  return out || undefined;
}

function checkNotShallow(repo) {
  const shallow = execFileSync("git", ["rev-parse", "--is-shallow-repository"], { cwd: repo, encoding: "utf8" }).trim();
  if (shallow === "true") {
    throw new Error("shallow clone: last-updated dates need full history (fetch-depth: 0), or set GHIRA_SITE_NO_DATES=1");
  }
}

function listDocs(dir, base = "") {
  const out = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const rel = base ? `${base}/${e.name}` : e.name;
    if (e.isDirectory()) out.push(...listDocs(join(dir, e.name), rel));
    else if (e.name.endsWith(".md")) out.push(`docs/${rel}`);
  }
  return out.sort();
}

/**
 * Files the build reads from outside apps/website besides the docs (the
 * design tokens and fonts, the sample meeting and the app's labels the
 * landing demo copies; tests compare against them).
 */
export const BUILD_INPUTS = [
  "packages/ui/src/tokens/tokens.css",
  "packages/ui/scripts/build-tokens-css.mjs",
  "packages/ui/src/fonts.css",
  "packages/ui/mocks/sample-meeting.json",
  "packages/i18n/locales/en.json",
  "packages/i18n/locales/vi.json",
  "tools/scripts/check-js-licenses.mjs",
];

/**
 * Repository files a doc links to (outside code fences), relative to the
 * repository root. The build fails when one is missing, so a change to them
 * must run the site build.
 */
export function linkedFiles(body, source, repo) {
  const text = body.replace(/^```[\s\S]*?^```/gm, "").replace(/`[^`\n]*`/g, "");
  const out = new Set();
  for (const m of text.matchAll(/\]\(([^)\s#]+)(?:#[^)\s]*)?\)|^\[[^\]]+\]:\s*(\S+)/gm)) {
    const url = m[1] ?? m[2];
    if (!url || /^[a-z][a-z0-9+.-]*:|^\/\//i.test(url)) continue;
    const path = posix.normalize(posix.join(repoDir(source), url.split("#")[0]));
    if (path.startsWith("..")) continue;
    if (existsSync(join(repo, path)) && statSync(join(repo, path)).isFile()) out.add(path);
  }
  return out;
}

const INDEX = {
  title: "Ghira documentation",
  description: "How to install Ghira, record meetings, and keep your notes on your Mac.",
};

export function sync({ repo = REPO, out = join(SITE, "content"), dates = !process.env.GHIRA_SITE_NO_DATES, log = console.log } = {}) {
  const inputs = new Set(["README.md", "docs/README.md", ...BUILD_INPUTS.filter((p) => existsSync(join(repo, p)))]);
  const nav = loadNav(repo);
  const published = publishedSources(nav);
  const quickstart = parseQuickstart(readFileSync(join(repo, "README.md"), "utf8"));
  if (dates) checkNotShallow(repo);

  const docsOut = join(out, "docs");
  rmSync(out, { recursive: true, force: true });
  mkdirSync(docsOut, { recursive: true });

  const navOut = [];
  for (const section of nav) {
    const pages = [];
    for (const { source, slug, description } of section.docs) {
      if (!SOURCE_PATH.test(source)) throw new Error(`${source}: not a valid docs path`);
      const file = join(repo, source);
      if (!existsSync(file)) throw new Error(`${source}: listed in the docs nav but missing`);
      inputs.add(source);
      const md = readFileSync(file, "utf8");
      const split = splitTitle(md, source);
      const body = expandQuickstart(split.body, quickstart);
      for (const linked of linkedFiles(body, source, repo)) inputs.add(linked);
      if (/!\[[^\]]*\]\(/.test(body.replace(/```[\s\S]*?```/g, ""))) throw new Error(`${source}: images are not supported on the site yet`);
      const data = { title: split.title, description, source };
      if (dates) {
        const date = lastUpdated(repo, source);
        if (date) data.lastUpdated = date;
      }
      const target = join(docsOut, contentPathOfSlug(slug));
      mkdirSync(dirname(target), { recursive: true });
      writeFileSync(target, frontmatter(data) + body);
      pages.push({ slug, title: split.title, description, source });
    }
    navOut.push({ title: section.title, pages });
  }

  // /docs: the index. Its cards are rendered from nav.json.
  writeFileSync(join(docsOut, "index.md"), frontmatter({ ...INDEX, source: "docs/README.md" }));

  // Sidebar order for fumadocs' page tree: sections as separators. A page in
  // a folder (release-notes/…) is listed by its path.
  const rootPages = ["index"];
  for (const s of navOut) rootPages.push(`---${s.title}---`, ...s.pages.map((p) => p.slug));
  writeFileSync(join(docsOut, "meta.json"), `${JSON.stringify({ title: "Docs", root: true, pages: rootPages }, null, 2)}\n`);

  // Generated data for the site.
  const gen = join(out, "generated");
  mkdirSync(gen, { recursive: true });
  writeFileSync(join(gen, "nav.json"), `${JSON.stringify({ index: INDEX, sections: navOut }, null, 2)}\n`);
  writeFileSync(join(gen, "quickstart.json"), `${JSON.stringify(quickstart, null, 2)}\n`);

  const unpublished = listDocs(join(repo, "docs")).filter((source) => source !== "docs/README.md" && !published.has(source));
  if (unpublished.length) log(`unpublished docs: ${unpublished.join(", ")}`);
  writeFileSync(join(gen, "inputs.json"), `${JSON.stringify([...inputs].sort(), null, 2)}\n`);
  log(`synced ${published.size - 1} docs into ${relative(SITE, docsOut)}`);
  return { published, unpublished, inputs, nav: navOut };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    sync();
  } catch (err) {
    console.error(`sync-docs: ${err.message}`);
    process.exit(1);
  }
}
