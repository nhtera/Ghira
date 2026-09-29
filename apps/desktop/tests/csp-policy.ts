// SPDX-License-Identifier: Apache-2.0
// Reads the production CSP from tauri.conf.json so tests check the real policy.
import { readFileSync } from "node:fs";

const confUrl = new URL("../src-tauri/tauri.conf.json", import.meta.url);

export function productionCsp(): string {
  const conf = JSON.parse(readFileSync(confUrl, "utf8"));
  return conf.app.security.csp as string;
}

/** Parses a CSP string into `directive -> sources`. */
export function parseCsp(csp: string): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const part of csp.split(";")) {
    const [name, ...sources] = part.trim().split(/\s+/);
    if (name) out.set(name, sources);
  }
  return out;
}
