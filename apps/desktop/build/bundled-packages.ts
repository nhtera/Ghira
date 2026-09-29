// SPDX-License-Identifier: Apache-2.0
// Vite plugin: records every npm package that ends up in the bundle, with its
// license, in dist/third-party-js.json. The license gate checks this list (what
// actually ships) instead of trusting the dependencies/devDependencies split,
// and the About screen can list the notices from it later.
import { readFileSync } from "node:fs";
import { join, sep } from "node:path";
import type { Plugin } from "vite";

type BundledPackage = { name: string; version: string; license: string };

function packageRoot(id: string): string | null {
  const marker = `${sep}node_modules${sep}`;
  const at = id.lastIndexOf(marker);
  if (at < 0) return null;
  const rest = id.slice(at + marker.length).split(sep);
  const depth = rest[0]?.startsWith("@") ? 2 : 1;
  return id.slice(0, at + marker.length) + rest.slice(0, depth).join(sep);
}

export function bundledPackages(): Plugin {
  return {
    name: "ghi-bundled-packages",
    apply: "build",
    generateBundle() {
      const found = new Map<string, BundledPackage>();
      for (const id of this.getModuleIds()) {
        const root = packageRoot(id.split("?")[0]);
        if (!root || found.has(root)) continue;
        const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
        found.set(root, { name: pkg.name, version: pkg.version, license: pkg.license ?? "UNKNOWN" });
      }
      const list = [...found.values()].sort((a, b) => a.name.localeCompare(b.name));
      this.emitFile({ type: "asset", fileName: "third-party-js.json", source: `${JSON.stringify(list, null, 2)}\n` });
    },
  };
}
