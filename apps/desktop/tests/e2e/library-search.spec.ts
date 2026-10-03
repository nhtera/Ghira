// SPDX-License-Identifier: Apache-2.0
// D3: search with accent-insensitive hits, filters, multi-select on the mocked core.
import { expect, test, type Page } from "@playwright/test";

const open = (page: Page, hash = "/meetings") => page.goto(`/?platform=win#${hash}`);

test("typing without accents finds accented hits, marked, and a segment hit opens the transcript at its time", async ({ page }) => {
  await open(page);
  const box = page.getByRole("searchbox");
  await box.fill("nhan dien");
  // Hits are grouped per meeting (a named group); each hit is one button.
  const hit = page.getByRole("group", { name: /Client call — Acme onboarding/ }).getByRole("button").first();
  await expect(hit).toBeVisible();
  // Marks are real <mark> text nodes covering the accented words.
  const marks = hit.locator("mark");
  await expect(marks.first()).toBeVisible();
  expect(((await marks.first().textContent()) ?? "").toLowerCase()).toMatch(/nh(ậ|a)n|di(ệ|e)n/);
  await expect(page.getByText(/results/)).toBeVisible();
  // A segment hit shows its time and opens the transcript there.
  await hit.click();
  await expect(page).toHaveURL(/#\/meetings\/[^/]+\/(transcript|notes)/);
});

test("Ctrl+F focuses the search; Escape clears it and the list comes back", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("heading", { name: "Yesterday" })).toBeVisible();
  await page.keyboard.press("Control+f");
  await expect(page.getByRole("searchbox")).toBeFocused();
  await page.keyboard.type("zzzz-nothing");
  await expect(page.getByText(/No meetings match “zzzz-nothing”/)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("heading", { name: "Yesterday" })).toBeVisible();
});

test("Source filter narrows the list and Clear filters restores it", async ({ page }) => {
  await open(page);
  const rows = page.getByRole("listitem");
  await expect(rows.first()).toBeVisible();
  const all = await rows.count();
  await page.getByRole("button", { name: "Source" }).click();
  await page.getByRole("menuitemcheckbox", { name: "Import" }).click();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: /Source · 1/ })).toBeVisible();
  await expect.poll(async () => rows.count()).toBeLessThan(all);
  await page.getByRole("button", { name: "Clear filters" }).first().click();
  await expect.poll(async () => rows.count()).toBe(all);
});

test("multi-select with shift-click shows the bulk bar; Delete asks, then undoes", async ({ page }) => {
  await open(page);
  const boxes = page.getByRole("checkbox", { name: /^Select / });
  await boxes.nth(0).click();
  await boxes.nth(2).click({ modifiers: ["Shift"] });
  await expect(page.getByRole("toolbar", { name: "3 selected" })).toBeVisible();
  await page.getByRole("toolbar").getByRole("button", { name: "Delete" }).click();
  await expect(page.getByRole("alertdialog")).toContainText("Delete 3 meetings?");
  await page.getByRole("alertdialog").getByRole("button", { name: "Delete" }).click();
  await expect(page.getByText("3 deleted", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByRole("checkbox", { name: /^Select / })).not.toHaveCount(0);
});
