// SPDX-License-Identifier: Apache-2.0
// Library, processing and the detection prompt on the mocked core.
import { expect, test, type Page } from "@playwright/test";

const open = (page: Page, hash = "/meetings") => page.goto(`/?platform=win#${hash}`);

test("lists the sample meetings grouped by day with statuses", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("heading", { name: "Yesterday" })).toBeVisible();
  await expect(page.getByRole("button", { name: /Client call — Acme onboarding/ })).toBeVisible();
  // The sample "final pass" row shows its percentage.
  await expect(page.getByText("Processing 62%")).toBeVisible();
  // Opening a row goes to the meeting (detail is phase 11).
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await expect(page).toHaveURL(/#\/meetings\/sample-\d+\/notes/);
});

test("quick actions show on keyboard focus and Delete asks first", async ({ page }) => {
  await open(page);
  const row = page.getByRole("listitem").filter({ hasText: "Client call — Acme onboarding" });
  await row.getByRole("button", { name: /Client call/ }).focus();
  await expect(row.getByRole("button", { name: "Open", exact: true })).toBeVisible();
  await row.getByRole("button", { name: "Delete" }).click();
  await expect(row).toContainText("Delete 1 meeting?");
  await row.getByRole("button", { name: "Delete" }).last().click();
  await expect(page.getByText("1 deleted", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: /Client call — Acme onboarding/ })).toHaveCount(0);
});

test("recording then stopping shows the stepper, the toast and Name your speakers", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "New recording" }).click();
  await expect(page.getByRole("heading", { name: "Live" })).toBeVisible();
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();
  const panel = page.getByRole("region", { name: "Writing your notes on this PC" });
  await expect(panel).toBeVisible();
  await expect(panel.getByRole("listitem").first()).toContainText("Reading the recording");
  await expect(panel).toContainText("You can leave this page");
  await expect(page.getByText("Notes are ready")).toBeVisible({ timeout: 6000 });
  await expect(panel).toHaveCount(0);
  const names = page.getByRole("region", { name: "Name your speakers" });
  await expect(names).toBeVisible();
  // The mock has no audio: a sample can't play but naming still works.
  await names.getByRole("button", { name: "Play 3 s" }).first().click();
  await names.getByRole("textbox").first().fill("Linh");
  await names.getByRole("button", { name: "Save" }).first().click();
  await expect(names.getByRole("listitem")).toHaveCount(2);
  await names.getByRole("button", { name: "Skip" }).click();
  await expect(names).toHaveCount(0);
});

test("the detection prompt offers Start without taking focus", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("heading", { name: "Meetings" })).toBeVisible();
  await page.evaluate(() => (window as unknown as { __ghiMock: { simulateMeetingDetected: () => void } }).__ghiMock.simulateMeetingDetected());
  const prompt = page.getByRole("region", { name: "Zoom call detected. Record it?" });
  await expect(prompt).toBeVisible();
  await expect(prompt.getByRole("button", { name: "Never for Zoom" })).toBeVisible();
  expect(await page.evaluate(() => document.activeElement === document.body)).toBe(true);
  await prompt.getByRole("button", { name: "Not now" }).click();
  await expect(prompt).toHaveCount(0);

  await page.evaluate(() => (window as unknown as { __ghiMock: { simulateMeetingDetected: () => void } }).__ghiMock.simulateMeetingDetected());
  await page.getByRole("region", { name: "Zoom call detected. Record it?" }).getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("heading", { name: "Live" })).toBeVisible();
});
