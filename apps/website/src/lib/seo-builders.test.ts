// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { faqPage, jsonForScript, softwareApplication } from "./json-ld.ts";
import { llmsTxt } from "./llms.ts";
import type { GeneratedNav } from "./site-links.ts";
import { robotsTxt, sitemapEntries, sitemapXml } from "./sitemap.ts";

const nav: GeneratedNav = {
  index: { title: "Ghira documentation", description: "d" },
  sections: [
    { title: "Get started", pages: [{ slug: "install", title: "Install from source", heading: "Install from source", description: "Build it", source: "docs/install.md", lastUpdated: "2026-10-08" }] },
    { title: "Project", pages: [{ slug: "release-notes/0-1-0-alpha-1", title: "Release notes", heading: "Ghira 0.1.0-alpha.1", description: "First alpha", source: "docs/release-notes/0-1-0-alpha-1.md" }] },
  ],
};

test("the sitemap is the landing page, the docs index and every nav page, no trailing slash", () => {
  assert.deepEqual(
    sitemapEntries(nav).map((e) => e.loc),
    ["https://ghira.app/", "https://ghira.app/docs", "https://ghira.app/docs/install", "https://ghira.app/docs/release-notes/0-1-0-alpha-1"],
  );
  const xml = sitemapXml(nav);
  assert.match(xml, /<loc>https:\/\/ghira\.app\/docs\/install<\/loc><lastmod>2026-10-08<\/lastmod>/);
  assert.ok(!xml.includes("og-card"));
  assert.match(robotsTxt(), /^User-agent: \*\nAllow: \/\n\nSitemap: https:\/\/ghira\.app\/sitemap\.xml\n$/);
});

test("llms.txt lists exactly the nav entries", () => {
  const txt = llmsTxt(nav, { name: "Ghira", summary: "Offline\nmeeting notes.", details: ["Pre-release."] });
  assert.ok(txt.startsWith("# Ghira\n\n> Offline meeting notes.\n\nPre-release.\n\n## Get started\n\n"));
  const links = [...txt.matchAll(/^- \[([^\]]+)\]\(([^)]+)\): (.+)$/gm)].map((m) => m[2]);
  assert.deepEqual(links, ["https://ghira.app/docs/install", "https://ghira.app/docs/release-notes/0-1-0-alpha-1"]);
});

test("the built nav and llms.txt agree (when synced)", () => {
  let built: GeneratedNav;
  try {
    built = JSON.parse(readFileSync(new URL("../../content/generated/nav.json", import.meta.url), "utf8"));
  } catch {
    return; // not synced in this checkout
  }
  const txt = llmsTxt(built, { name: "Ghira", summary: "s", details: [] });
  assert.equal([...txt.matchAll(/^- \[/gm)].length, built.sections.flatMap((s) => s.pages).length);
});

test("JSON-LD: FAQPage mirrors the FAQ data; script text cannot close the element", () => {
  const faq = [{ q: "Does it <script> & stuff?", a: "No </script><script>alert(1)</script>" }];
  const page = faqPage(faq);
  assert.equal(page.mainEntity[0].name, faq[0].q);
  assert.equal(page.mainEntity[0].acceptedAnswer.text, faq[0].a);
  const text = jsonForScript([softwareApplication({ name: "Ghira", description: "x" }), page]);
  assert.ok(!text.includes("<") && !text.includes(">") && !text.includes("&"));
  assert.deepEqual(JSON.parse(text)[1], page);
  assert.equal(JSON.parse(text)[0].offers.price, "0");
});
