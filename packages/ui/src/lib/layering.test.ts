// SPDX-License-Identifier: Apache-2.0
// The helpers in lib/ are shared by both apps, so they stay pure: no React, no
// app bindings, nothing from apps/*, no Tauri, no i18n runtime. A file that
// grows such an import fails here.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const dir = dirname(fileURLToPath(import.meta.url));
const files = readdirSync(dir).filter((f) => f.endsWith(".ts") && !f.endsWith(".test.ts"));

const FORBIDDEN = [
  /^react(\/|$)/,
  /^react-dom(\/|$)/,
  /^@tauri-apps\//,
  /^@tanstack\//,
  /^react-i18next$/,
  /^i18next$/,
  /bindings/,
  /(^|\/)apps\//,
  /^@ghi\/(desktop|mobile)(\/|$)/,
];

/** Every module specifier a file imports or re-exports from (static, dynamic, require). */
export function importsOf(source: string): string[] {
  const out: string[] = [];
  const re = /(?:\bfrom\s*|\bimport\s*\(?\s*|\brequire\s*\(\s*)(["'])([^"']+)\1/g;
  for (let m = re.exec(source); m; m = re.exec(source)) out.push(m[2]!);
  return out;
}

describe("lib/ stays React-free and binding-free", () => {
  it("has files to check", () => {
    expect(files).toEqual(expect.arrayContaining(["notes-tree.ts", "talk-share.ts"]));
  });

  for (const f of files)
    it(`${f} imports nothing from React, the apps or their bindings`, () => {
      const bad = importsOf(readFileSync(join(dir, f), "utf8")).filter((spec) => FORBIDDEN.some((r) => r.test(spec)));
      expect(bad).toEqual([]);
    });

  it("the check itself catches what it should", () => {
    const src = `import { useState } from "react";\nimport type { MeetingNotes } from "../../bindings";\nexport * from "../../../apps/desktop/x";\nconst a = await import("react-dom");\nimport { ok } from "./talk-share";`;
    expect(importsOf(src).filter((s) => FORBIDDEN.some((r) => r.test(s)))).toEqual(["react", "../../bindings", "../../../apps/desktop/x", "react-dom"]);
  });
});
