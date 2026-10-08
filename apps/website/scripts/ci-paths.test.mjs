// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import { test } from "node:test";
import { matches, sitePaths } from "./ci-paths.mjs";

test("the workflow's path filter is read without a YAML parser", () => {
  const globs = sitePaths("env:\n  NODE: x\n  SITE_PATHS: |\n    apps/website/**\n    # a note\n    README.md\n\njobs:\n  x: y\n");
  assert.deepEqual(globs, ["apps/website/**", "README.md"]);
  for (const p of ["apps/website/**", "docs/**", "PRIVACY.md", "packages/ui/src/tokens/**", ".github/workflows/site.yml"]) assert.ok(sitePaths().includes(p), p);
});

test("globs match folders and exact files", () => {
  const globs = ["apps/website/**", "README.md", "packages/ui/src/tokens/**"];
  assert.ok(matches("apps/website/src/x.ts", globs));
  assert.ok(matches("packages/ui/src/tokens/tokens.css", globs));
  assert.ok(matches("README.md", globs));
  assert.ok(!matches("docs/README.md", globs));
  assert.ok(!matches("apps/websites/x", globs));
  assert.ok(!matches("crates/ghi-net/src/lib.rs", globs));
});
