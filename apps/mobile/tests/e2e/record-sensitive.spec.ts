// SPDX-License-Identifier: Apache-2.0
// Sensitive meeting mode and "discard the last N minutes" on the record screen
// (scripted mock): armed before the start, turned on mid-recording, refused
// without a live transcript; discard previews before it removes anything.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible } from "./helpers";
import { log, openRecord, startRecording } from "./record-support";

const addLines = (page: Page, n: number) => page.evaluate((x) => window.__ghiRecord!.addLines(x), n);
const lines = (page: Page) => page.getByTestId("line");

async function live(page: Page) {
  await startRecording(page, "Record room");
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await expect(page.getByText("Getting ready")).toHaveCount(0);
}

test("chosen in the start sheet: the recording is sensitive from its first second", async ({ page }) => {
  await openRecord(page);
  await page.getByRole("button", { name: "Record room" }).click();
  const sw = page.getByRole("switch", { name: "Sensitive meeting" });
  await expect(sw).not.toBeChecked();
  await expect(page.getByText(/no audio is saved, nothing goes to the cloud/)).toBeVisible();
  await expectAccessible(page);
  await sw.click();
  await expect(sw).toBeChecked();
  await page.getByRole("button", { name: "Everyone knows, start recording" }).click();

  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  await expect(page.getByText("Getting ready")).toHaveCount(0);
  expect(await page.evaluate(() => window.__ghiRecord!.lastStart)).toMatchObject({ sensitive: true });
  await expect(page.getByTestId("sensitive-badge")).toHaveText("Sensitive · no audio kept");
  await addLines(page, 2);
  await expect(lines(page)).toHaveCount(2);

  // It stays on until the recording ends.
  await page.getByRole("button", { name: "More actions" }).click();
  const on = page.getByRole("button", { name: /Sensitive meeting · On until the recording ends/ });
  await expect(on).toBeDisabled();
  await page.getByRole("button", { name: "Close" }).first().click();

  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("button", { name: "Record room" })).toBeVisible();
  // The choice was for that one recording: the next start sheet is off again.
  await page.getByRole("button", { name: "Record room" }).click();
  await expect(page.getByRole("switch", { name: "Sensitive meeting" })).not.toBeChecked();
});

test("turned on mid-recording: asks first, says it can't be undone, then shows the badge", async ({ page }) => {
  await openRecord(page);
  await live(page);
  await expect(page.getByTestId("sensitive-badge")).toHaveCount(0);

  await page.getByRole("button", { name: "More actions" }).click();
  await page.getByRole("button", { name: "Make sensitive…" }).click();
  const sheet = page.getByRole("dialog", { name: "Make this meeting sensitive?" });
  await expect(sheet).toContainText("deleted when you stop");
  await expect(sheet).toContainText("can’t be turned off during this recording");
  await expectAccessible(page);

  // Cancel changes nothing.
  await sheet.getByRole("button", { name: "Cancel" }).click();
  expect(await log(page)).not.toContain("recordSetSensitive:true");
  await expect(page.getByTestId("sensitive-badge")).toHaveCount(0);

  await page.getByRole("button", { name: "More actions" }).click();
  await page.getByRole("button", { name: "Make sensitive…" }).click();
  await page.getByRole("dialog", { name: "Make this meeting sensitive?" }).getByRole("button", { name: "Make sensitive" }).click();
  await expect(page.getByTestId("sensitive-badge")).toBeVisible();
  expect(await log(page)).toContain("recordSetSensitive:true");
});

test("below the live tier there is nothing to keep, so the choice is not offered", async ({ page }) => {
  await openRecord(page, { knobs: { tier: "recordOnly" } });
  await page.getByRole("button", { name: "Record room" }).click();
  await expect(page.getByRole("dialog", { name: "Tell everyone you’re recording" })).toBeVisible();
  await expect(page.getByRole("switch", { name: "Sensitive meeting" })).toHaveCount(0);
});

test("a refused sensitive start says so and records nothing", async ({ page }) => {
  await openRecord(page);
  await page.getByRole("button", { name: "Record room" }).click();
  await page.getByRole("switch", { name: "Sensitive meeting" }).click();
  await page.evaluate(() => Object.assign(window.__ghiRecord as object, { failStart: "sensitiveNeedsTranscript" }));
  await page.getByRole("button", { name: "Everyone knows, start recording" }).click();
  await expect(page.getByText(/Sensitive mode needs the live transcript/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Record room" })).toBeVisible();
});

test("discard the last minutes: previews what goes, then removes exactly that", async ({ page }) => {
  await openRecord(page);
  await live(page);
  await addLines(page, 4);
  await expect(lines(page)).toHaveCount(4);
  await page.getByRole("button", { name: "Mark" }).click();

  await page.getByRole("button", { name: "More actions" }).click();
  await page.getByRole("button", { name: "Discard the last 1 minute…" }).click();
  const sheet = page.getByRole("dialog", { name: "Discard the last 1 minute?" });
  await expect(sheet.getByTestId("discard-preview")).toContainText("This removes the audio and 4 transcript lines, 1 mark.");
  await expectAccessible(page);

  // Cancel removes nothing.
  await sheet.getByRole("button", { name: "Cancel" }).click();
  await expect(lines(page)).toHaveCount(4);
  expect((await log(page)).some((l) => l.startsWith("recordDiscardFrom"))).toBe(false);

  await page.getByRole("button", { name: "More actions" }).click();
  await page.getByRole("button", { name: "Discard the last 1 minute…" }).click();
  await page.getByRole("dialog", { name: "Discard the last 1 minute?" }).getByRole("button", { name: "Discard" }).click();
  await expect(lines(page)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Mark", exact: true })).toBeVisible();
  // The cut it showed is the cut it made.
  expect(await log(page)).toEqual(expect.arrayContaining(["recordDiscardPreview:60", "recordDiscardFrom:0"]));
  // Recording goes on.
  await addLines(page, 1);
  await expect(lines(page)).toHaveCount(1);
});
