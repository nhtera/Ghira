// SPDX-License-Identifier: Apache-2.0
// D1 onboarding on the mocked core. Copy that the lead hasn't merged yet
// (PENDING-ui-onboarding.json) shows as its key, so steps are told apart by
// the rail's current item, not by their text.
import { expect, test, type Page } from "@playwright/test";

const open = (page: Page, step = "welcome") => page.goto(`/?platform=win#/onboarding/${step}`);
const current = (page: Page) => page.locator("[aria-current=step] > span").first();
const primary = (page: Page) => page.locator("[data-onboarding-primary]");

test("walks every step and lands in the library", async ({ page }) => {
  test.setTimeout(45_000);
  await open(page);
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Meeting notes that stay on this PC");
  await expect(current(page)).toHaveText("1");
  await page.getByRole("button", { name: "Get started" }).click();

  // Languages: a radio group; the choice is kept.
  await expect(page.getByRole("heading", { name: "Which languages do you speak in meetings?" })).toBeVisible();
  await page.getByRole("radio", { name: "Tiếng Việt" }).click();
  await expect(page.getByRole("radio", { name: "Tiếng Việt" })).toHaveAttribute("aria-checked", "true");
  await page.getByRole("button", { name: "Continue" }).click();

  // Models (installed on the mock): done, rows listed, never blocking.
  await expect(page.getByRole("heading", { name: "Download speech models" })).toBeVisible();
  await expect(page.getByText("Downloaded and checked").first()).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();

  // Permissions: the mock's mic is allowed.
  await expect(page.getByRole("heading", { name: /to hear your meetings/ })).toBeVisible();
  await expect(page.getByText("Allowed")).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();

  // Your voice: optional, Skip is always there (the real enrollment has its own spec).
  await expect(current(page)).toHaveText("5");
  await expect(page.getByRole("heading", { name: /Teach .+ your voice/ })).toBeVisible();
  await page.getByRole("button", { name: "Skip" }).click();

  // Test recording.
  await expect(current(page)).toHaveText("6");
  await expect(page.getByRole("heading", { name: "Try a 10-second test" })).toBeVisible();
  await page.getByRole("button", { name: "Run test" }).click();
  await expect(page.getByRole("meter", { name: "Mic" })).toBeVisible();
  await expect(page.getByText("Both sources are working")).toBeVisible({ timeout: 15_000 });
  await page.getByRole("button", { name: "Continue" }).click();

  // Recovery key: optional, never blocks.
  await expect(current(page)).toHaveText("7");
  await page.getByRole("button", { name: /^(Set up later|onboarding\.recovery\.later)$/ }).click();

  // Done: shortcuts with Ctrl labels, finish.
  await expect(page.getByRole("heading", { name: "You’re ready for your next call" })).toBeVisible();
  await expect(page.getByText("Ctrl+Shift+R")).toBeVisible();
  await primary(page).click();
  await expect(page).toHaveURL(/#\/meetings$/);
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();
});

test("Enter continues, Escape stays, Back goes back", async ({ page }) => {
  await open(page, "languages");
  // The step's keys work once it has rendered.
  await expect(page.getByRole("radio", { checked: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page).toHaveURL(/onboarding\/languages/);
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/onboarding\/models/);
  await page.getByRole("button", { name: "Back" }).click();
  await expect(page).toHaveURL(/onboarding\/languages/);
  // Arrow keys move through the language radios.
  await page.getByRole("radio", { checked: true }).focus();
  await page.keyboard.press("ArrowUp");
  await expect(page.getByRole("radio", { name: "Tiếng Việt" })).toBeFocused();
});

test("the voice step is in the rail, and a typed URL for it lands on it", async ({ page }) => {
  await open(page, "permissions");
  await expect(page.getByRole("navigation").getByRole("listitem")).toHaveCount(8);
  await expect(page.getByRole("navigation").getByText("Your voice")).toHaveCount(1);
  await open(page, "voice");
  await expect(current(page)).toHaveText("5");
});

test("fits the compact window on every step", async ({ page }) => {
  test.setTimeout(30_000);
  await page.setViewportSize({ width: 960, height: 640 });
  await open(page);
  const next = [
    () => primary(page),
    () => primary(page),
    () => primary(page),
    () => primary(page),
    () => page.getByRole("button", { name: "Skip" }),
    () => page.getByRole("button", { name: "Skip" }),
    () => page.getByRole("button", { name: /^(Set up later|onboarding\.recovery\.later)$/ }),
  ];
  for (let i = 0; i <= next.length; i++) {
    await expect(current(page)).toHaveText(String(i + 1));
    const overflow = await page.evaluate(() => ({ x: document.documentElement.scrollWidth - window.innerWidth, y: document.documentElement.scrollHeight - window.innerHeight }));
    expect(overflow.x, `step ${i + 1} overflows sideways`).toBeLessThanOrEqual(0);
    expect(overflow.y, `step ${i + 1} grows the page (it scrolls inside)`).toBeLessThanOrEqual(0);
    if (i < next.length) await next[i]!().click();
  }
});
