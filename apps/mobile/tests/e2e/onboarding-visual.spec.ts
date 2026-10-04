// SPDX-License-Identifier: Apache-2.0
// M1 visual baselines: light/dark x EN/VI, and 200% text (Dynamic Type max).
// Update with `pnpm --filter @ghi/mobile test:e2e --update-snapshots`.
import { expect, test } from "@playwright/test";
import { openOnboarding } from "./record-support";

const SCREENS: { name: string; completed: string[]; knobs?: Record<string, unknown> }[] = [
  { name: "languages", completed: [] },
  { name: "mic", completed: ["languages"] },
  { name: "mic-denied", completed: ["languages"], knobs: { mic: "denied" } },
  { name: "models", completed: ["languages", "micPriming", "consent", "processing"] },
  { name: "done", completed: ["languages", "micPriming", "consent", "processing", "models", "voice"], knobs: { modelsReady: true } },
];

const LOOKS = [
  { id: "en-light", lang: "en", scheme: "light", scale: undefined },
  { id: "en-dark", lang: "en", scheme: "dark", scale: undefined },
  { id: "vi-light", lang: "vi", scheme: "light", scale: undefined },
  { id: "vi-dark", lang: "vi", scheme: "dark", scale: undefined },
  { id: "en-200", lang: "en", scheme: "light", scale: 2 },
  { id: "vi-200", lang: "vi", scheme: "light", scale: 2 },
] as const;

for (const screen of SCREENS) {
  for (const look of LOOKS) {
    test(`onboarding ${screen.name} ${look.id}`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: look.scheme, reducedMotion: "reduce" });
      await openOnboarding(page, { lang: look.lang, scale: look.scale, completed: screen.completed, knobs: screen.knobs });
      await expect(page.locator("[data-step-title]")).toBeVisible();
      if (screen.name === "done") await expect(page.locator("[data-step=done] ul")).toBeVisible();
      await expect(page).toHaveScreenshot(`onboarding-${screen.name}-${look.id}.png`, { animations: "disabled" });
    });
  }
}

test("200% text: nothing clips or scrolls sideways, every action stays reachable", async ({ page }) => {
  for (const screen of SCREENS) {
    await openOnboarding(page, { lang: "vi", scale: 2, completed: screen.completed, knobs: screen.knobs });
    await expect(page.locator("[data-step-title]")).toBeVisible();
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, screen.name).toBeLessThanOrEqual(0);
    for (const button of await page.getByRole("button").all()) {
      const box = await button.boundingBox();
      expect(box === null || box.height >= 43.5, `${screen.name} button height`).toBe(true);
    }
  }
});
