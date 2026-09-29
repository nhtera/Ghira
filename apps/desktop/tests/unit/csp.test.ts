// SPDX-License-Identifier: Apache-2.0
// RT-6: the production CSP allows no remote origins and no inline/eval scripts.
import { describe, expect, it } from "vitest";
import { parseCsp, productionCsp } from "../csp-policy";

const csp = parseCsp(productionCsp());
// Custom schemes appear twice: `x:` on macOS/iOS/Linux, `http://x.localhost` on Windows/Android.
const allowedSchemes = new Set([
  "'self'",
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

  it("never allows unsafe-inline or unsafe-eval", () => {
    expect(productionCsp()).not.toMatch(/unsafe-(inline|eval)/);
  });
});
