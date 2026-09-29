// SPDX-License-Identifier: Apache-2.0
// Fails if any shipped npm package uses a license outside the allow-list
// (docs/05 §4.1). Checks two lists:
//   1. production dependencies of every workspace package (pnpm's own report);
//   2. every package actually bundled into the desktop frontend
//      (apps/desktop/dist/third-party-js.json, written by `pnpm build`), which
//      catches devDependencies that end up in the bundle.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const satisfies = require("spdx-satisfies");

const allowed = [
  "Apache-2.0", "MIT", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib",
  "MPL-2.0", "OFL-1.1", "Unicode-3.0", "CC0-1.0",
];

function isAllowed(expression) {
  try {
    return satisfies(expression, allowed);
  } catch {
    return false; // not a valid SPDX expression, e.g. "UNKNOWN" or "SEE LICENSE IN ..."
  }
}

const bad = [];

const report = JSON.parse(
  execFileSync("pnpm", ["licenses", "list", "--prod", "--recursive", "--json"], {
    encoding: "utf8",
    shell: process.platform === "win32",
  }),
);
for (const [license, packages] of Object.entries(report)) {
  if (isAllowed(license)) continue;
  for (const pkg of packages) bad.push(`${pkg.name}@${(pkg.versions ?? [pkg.version]).join(",")}: ${license}`);
}

const bundleList = new URL("../../apps/desktop/dist/third-party-js.json", import.meta.url);
if (!existsSync(bundleList)) {
  console.error("apps/desktop/dist/third-party-js.json is missing: run `pnpm build` first.");
  process.exit(1);
}
const bundled = JSON.parse(readFileSync(bundleList, "utf8"));
for (const pkg of bundled) {
  if (!isAllowed(pkg.license)) bad.push(`${pkg.name}@${pkg.version} (bundled): ${pkg.license}`);
}

if (bad.length) {
  console.error("npm packages with licenses outside the allow-list:");
  for (const line of bad) console.error(`  ${line}`);
  process.exit(1);
}
console.log(`js licenses: ok (${Object.keys(report).length} license kinds in prod deps; ${bundled.length} bundled packages)`);
