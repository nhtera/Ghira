// SPDX-License-Identifier: Apache-2.0
// RT-6: the production CSP allows no remote origins and no inline/eval scripts.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { parseCsp, productionCsp } from "../csp-policy";

const csp = parseCsp(productionCsp());
// Custom schemes appear twice: `x:` on macOS/iOS/Linux, `http://x.localhost` on Windows/Android.
const allowedSchemes = new Set([
  "'self'",
  "'unsafe-inline'",
  "'none'",
  "data:",
  "ipc:",
  "http://ipc.localhost",
  "ghi-audio:",
  "http://ghi-audio.localhost",
]);

describe("production CSP", () => {
  it("defaults to self and blocks plugins, forms, frames and <base>", () => {
    expect(csp.get("default-src")).toEqual(["'self'"]);
    // These directives do not fall back to default-src (or need to be stricter).
    for (const directive of ["object-src", "form-action", "base-uri", "frame-src"]) {
      expect(csp.get(directive), directive).toEqual(["'none'"]);
    }
  });

  it("lists only local sources", () => {
    for (const [directive, sources] of csp) {
      for (const source of sources) {
        expect(allowedSchemes, `${directive} ${source}`).toContain(source);
      }
    }
  });

  it("never allows inline or eval'd scripts", () => {
    expect(productionCsp()).not.toMatch(/unsafe-eval/);
    expect(csp.get("script-src") ?? csp.get("default-src")).toEqual(["'self'"]);
  });

  // Radix's scroll lock injects a <style> element, so styles may be inline.
  // Injecting styles needs script execution first, and nothing can leave
  // through CSS (img/font/connect stay local). RT-6 still bans HTML rendering.
  it("allows inline styles only in style-src", () => {
    for (const [directive, sources] of csp) {
      if (directive !== "style-src") expect(sources, directive).not.toContain("'unsafe-inline'");
    }
    expect(csp.get("style-src")).toEqual(["'self'", "'unsafe-inline'"]);
  });

  // An inline <style> in index.html makes Tauri add a style nonce, and with a
  // nonce browsers ignore 'unsafe-inline': every injected style would break.
  it("index.html has no inline <style>", () => {
    expect(readFileSync(new URL("../../index.html", import.meta.url), "utf8")).not.toMatch(/<style/i);
  });
});
