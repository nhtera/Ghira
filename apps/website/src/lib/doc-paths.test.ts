// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { test } from "node:test";
import { checkDocName, contentPathOfSlug, docUrlOfSource, editUrl, githubUrl, slugOfContentPath, slugOfSource, sourceOfNavHref, splitAnchor } from "./doc-paths.ts";
import { absolute, doc } from "./urls.ts";

test("slugs: root docs are lowercase, the index is empty, no trailing slash", () => {
  assert.equal(slugOfSource("docs/notes.md"), "notes");
  assert.equal(slugOfSource("docs/release-notes/0-1-0-alpha-1.md"), "release-notes/0-1-0-alpha-1");
  assert.equal(slugOfSource("PRIVACY.md"), "privacy");
  assert.equal(slugOfSource("docs/README.md"), "");
  assert.equal(docUrlOfSource("SECURITY.md", "reporting"), "/docs/security#reporting");
  assert.equal(doc("x/", "#a"), "/docs/x#a");
  assert.equal(absolute("/docs/x/"), "https://ghira.app/docs/x");
  assert.equal(absolute("/"), "https://ghira.app/");
});

test("content paths round-trip", () => {
  for (const slug of ["", "notes", "privacy", "release-notes/0-1-0-alpha-1"]) {
    assert.equal(slugOfContentPath(contentPathOfSlug(slug)), slug);
  }
  assert.equal(contentPathOfSlug(""), "index.md");
});

test("nav hrefs: docs pages and the root allowlist only, never docs/release", () => {
  assert.equal(sourceOfNavHref("notes.md"), "docs/notes.md");
  assert.equal(sourceOfNavHref("release-notes/0-1-0-alpha-1.md"), "docs/release-notes/0-1-0-alpha-1.md");
  for (const root of ["PRIVACY", "SECURITY", "CONTRIBUTING", "TRADEMARKS"]) assert.equal(sourceOfNavHref(`../${root}.md`), `${root}.md`);
  for (const bad of ["../LICENSE", "../README.md", "../CODE_OF_CONDUCT.md", "../tools/eval/README.md", "../../x.md"]) {
    assert.throws(() => sourceOfNavHref(bad), /outside docs/, bad);
  }
  assert.throws(() => sourceOfNavHref("release/smoke-checklist.md"), /never published/);
  assert.throws(() => sourceOfNavHref("README.md"), /docs index/);
  assert.throws(() => sourceOfNavHref("release-notes/v1.0.en.md"), /file name/);
  assert.throws(() => sourceOfNavHref("Notes.md"), /file name/);
});

test("file names are checked", () => {
  checkDocName("getting-started.md");
  checkDocName("release-notes/0-1-0-alpha-1.md");
  assert.throws(() => checkDocName("Guide.md"), /file name/);
  assert.throws(() => checkDocName("a b.md"), /file name/);
  assert.throws(() => checkDocName("Guides/x.md"), /folder name/);
});

test("GitHub URLs and the edit URL", () => {
  assert.equal(githubUrl("SECURITY.md"), "https://github.com/nhtera/Ghira/blob/main/SECURITY.md");
  assert.equal(githubUrl("README.md", { anchor: "install" }), "https://github.com/nhtera/Ghira/blob/main/README.md#install");
  assert.equal(githubUrl("docs/release", { dir: true }), "https://github.com/nhtera/Ghira/tree/main/docs/release");
  assert.equal(editUrl("docs/notes.md"), "https://github.com/nhtera/Ghira/edit/main/docs/notes.md");
  assert.equal(editUrl("PRIVACY.md"), "https://github.com/nhtera/Ghira/edit/main/PRIVACY.md");
  for (const bad of ["docs/../../etc/passwd", "docs/../x.md", "docs/./x.md", "https://evil.example/x.md", "LICENSE.md", "docs/release/smoke-checklist.md"]) {
    assert.throws(() => editUrl(bad), /not a docs source path/, bad);
  }
});

test("anchors split off", () => {
  assert.deepEqual(splitAnchor("x.md#a"), { path: "x.md", anchor: "a" });
  assert.deepEqual(splitAnchor("x.md"), { path: "x.md" });
  assert.deepEqual(splitAnchor("x.md#"), { path: "x.md", anchor: undefined });
});
