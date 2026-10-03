// SPDX-License-Identifier: Apache-2.0
// The menu bar popover and the mini recorder pages on the mocked core.
import { expect, test, type Page } from "@playwright/test";

const open = (page: Page, hash: string) => page.goto(`/?platform=win#${hash}`);

test("popover: idle shows Record, recent meetings and the privacy line", async ({ page }) => {
  await open(page, "/popover");
  await expect(page.getByRole("button", { name: "Record call" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Recent" }).getByRole("listitem")).toHaveCount(3);
  await expect(page.getByText("Local only")).toBeVisible();
  // No shell: no sidebar, and the page is transparent outside the card.
  await expect(page.getByRole("navigation", { name: "Main" })).toHaveCount(0);
  await expect(page.locator("html")).toHaveAttribute("data-window", "panel");
  // Opaque native window: the card fills it edge to edge (no margin, no radius).
  const box = await page.getByRole("main").boundingBox();
  const view = page.viewportSize()!;
  expect(box).toMatchObject({ x: 0, y: 0, width: view.width, height: view.height });
});

test("popover: Record starts a call; the card then shows the recording", async ({ page }) => {
  await open(page, "/popover");
  await page.getByRole("button", { name: "Record call" }).click();
  const card = page.getByRole("region", { name: "Recording" });
  await expect(card).toBeVisible();
  await expect(card.getByRole("button", { name: "Pause" })).toBeVisible();
  await card.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("button", { name: "Record call" })).toBeVisible();
});

test("popover: the first Tab stop is Record call, then Record room, then the meetings", async ({ page }) => {
  await open(page, "/popover");
  const record = page.getByRole("button", { name: "Record call" });
  await expect(record).toBeVisible();
  // Nothing is focused on open (the panel never steals focus); Tab starts at Record.
  expect(await page.evaluate(() => document.activeElement === document.body)).toBe(true);
  await page.keyboard.press("Tab");
  await expect(record).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: "Record room" })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("region", { name: "Recent" }).getByRole("button").first()).toBeFocused();
});

test("mini: controls a recording and collapses to a pill", async ({ page }) => {
  await open(page, "/popover");
  await page.getByRole("button", { name: "Record call" }).click();
  await expect(page.getByRole("region", { name: "Recording" })).toBeVisible();
  // The mock is one JS context per page: reuse it by switching route.
  await page.evaluate(() => (window.location.hash = "#/mini"));
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await expect(page.getByRole("meter", { name: "Mic" })).toBeVisible();
  await page.getByRole("button", { name: "Mark moment" }).click();
  await page.getByRole("button", { name: "Pause" }).click();
  await expect(page.getByRole("button", { name: "Resume" })).toBeVisible();
  await page.getByRole("button", { name: "Shrink to a pill" }).click();
  await expect(page.getByRole("button", { name: "Expand" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Stop" })).toHaveCount(0);
  await page.getByRole("button", { name: "Expand" }).click();
  await page.getByRole("button", { name: "Stop" }).click();
});

test("detect panel: Start records, opens the mini recorder and closes", async ({ page }) => {
  await open(page, "/detect?app=zoom&name=Zoom&browser=0");
  await expect(page.getByRole("region", { name: "Zoom call detected. Record it?" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Main" })).toHaveCount(0);
  expect(await page.evaluate(() => document.activeElement === document.body)).toBe(true);
  await page.getByRole("button", { name: "Start" }).click();
  // The mock core is now recording; the panel's own page just stays rendered.
  await page.evaluate(() => (window.location.hash = "#/popover"));
  await expect(page.getByRole("region", { name: "Recording" })).toBeVisible();
});

test("mini: #/mini?compact=1 opens as a pill without the line", async ({ page }) => {
  await open(page, "/popover");
  await page.getByRole("button", { name: "Record call" }).click();
  await expect(page.getByRole("region", { name: "Recording" })).toBeVisible();
  await page.evaluate(() => (window.location.hash = "#/mini?compact=1"));
  await expect(page.getByRole("button", { name: "Expand" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Stop" })).toHaveCount(0);
});
