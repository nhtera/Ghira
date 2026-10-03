// SPDX-License-Identifier: Apache-2.0
// Every component state renders under the production CSP with no violation
// (catches libraries that inject inline styles or fetch anything).
import { expect, test } from "@playwright/test";
import { galleryEntries, isPhone, storyUrl } from "./gallery";

test("no CSP violations across every story and state", async ({ page }) => {
  test.setTimeout(10 * 60_000);
  const entries = await galleryEntries(page);
  expect(entries.length).toBeGreaterThan(0);
  await page.addInitScript(() => {
    (window as unknown as { __csp: string[] }).__csp = [];
    document.addEventListener("securitypolicyviolation", (e) =>
      (window as unknown as { __csp: string[] }).__csp.push(`${e.effectiveDirective} ${e.blockedURI}`),
    );
  });
  const violations: string[] = [];
  for (const e of entries) {
    for (const state of e.states) {
      for (const platform of isPhone(e) ? ["ios"] : ["mac", "win"]) {
        await page.goto(storyUrl(e.id, { theme: "dark", lang: "vi" }, { state, platform }));
        await page.waitForLoadState("load");
        const seen = await page.evaluate(() => (window as unknown as { __csp: string[] }).__csp);
        violations.push(...seen.map((v) => `${e.id}/${state}/${platform}: ${v}`));
      }
    }
  }
  expect(violations).toEqual([]);
});
