// SPDX-License-Identifier: Apache-2.0
// The helpers in lib/ are shared by both apps, so they stay pure. The rule is an allowlist:
// a file in lib/ (any depth, .ts or .tsx) imports only its siblings by a relative `./` path,
// never a package (React, Tauri, i18n, an app's bindings) and never a path that climbs out.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const dir = dirname(fileURLToPath(import.meta.url));

function sources(root: string): string[] {
  return readdirSync(root, { withFileTypes: true }).flatMap((e) => {
    const p = join(root, e.name);
    if (e.isDirectory()) return sources(p);
    return /\.tsx?$/.test(e.name) && !/\.test\.tsx?$/.test(e.name) ? [p] : [];
  });
}

/** Every module specifier a file imports or re-exports from (static, dynamic, require). */
export function importsOf(source: string): string[] {
  const out: string[] = [];
  const re = /(?:\bfrom\s*|\bimport\s*\(?\s*|\brequire\s*\(\s*)(["'])([^"']+)\1/g;
  for (let m = re.exec(source); m; m = re.exec(source)) out.push(m[2]!);
  return out;
}

/** What is not allowed: anything but a `./` sibling that stays inside the folder. */
export function outsiders(specs: string[]): string[] {
  return specs.filter((s) => !s.startsWith("./") || s.split("/").includes(".."));
}

describe("lib/ stays pure: siblings only", () => {
  const files = sources(dir);

  it("has files to check", () => {
    expect(files.map((f) => relative(dir, f))).toEqual(expect.arrayContaining(["notes-tree.ts", "talk-share.ts"]));
  });

  for (const f of files)
    it(`${relative(dir, f)} imports only its siblings`, () => {
      expect(outsiders(importsOf(readFileSync(f, "utf8")))).toEqual([]);
    });

  it("the check itself catches what it should", () => {
    const bad = [
      `import { useState } from "react";`,
      `import type { MeetingNotes } from "../../bindings";`,
      `export * from "../../../apps/desktop/x";`,
      `const a = await import("react-dom");`,
      `import "@tauri-apps/api/core";`,
      `import x from "./../escape";`,
      `const r = require("i18next");`,
    ];
    for (const line of bad) expect(outsiders(importsOf(line)), line).toHaveLength(1);
    expect(outsiders(importsOf(`import { ok } from "./talk-share";\nexport * from "./sub/x";`))).toEqual([]);
  });
});
