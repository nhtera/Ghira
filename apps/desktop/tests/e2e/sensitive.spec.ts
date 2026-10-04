// SPDX-License-Identifier: Apache-2.0
// Sensitive meeting mode on the mocked core: armed before the start, turned on
// from the live More menu (asks first, one way), and set on a stored meeting
// (asks first, the audio goes, playback and cloud go with it).
import { expect, test, type Page } from "@playwright/test";

async function record(page: Page) {
  await page.goto("/?platform=win#/live");
  await expect(page.getByTestId("sensitive-start")).toBeVisible();
}
const live = async (page: Page) => {
  await expect(page.getByRole("textbox", { name: "Meeting title" })).toBeVisible();
  await expect(page.getByRole("main").locator("ol > li").first()).toContainText("Okay, bắt đầu nhé.", { timeout: 5000 });
};

test("armed before the start: the recording is sensitive and says so", async ({ page }) => {
  await record(page);
  const toggle = page.getByTestId("sensitive-start");
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await page.getByRole("button", { name: "Record call", exact: true }).click();
  await live(page);
  await expect(page.getByTestId("sensitive-badge")).toHaveText("Sensitive · no audio kept");

  // It stays on until the recording ends.
  await page.getByRole("button", { name: "More actions" }).click();
  await expect(page.getByRole("menuitem", { name: /Sensitive meeting/ })).toBeDisabled();
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: "Stop" }).click();
  await page.goto("/?platform=win#/live");
  // The switch was for that one recording.
  await expect(page.getByTestId("sensitive-start")).toHaveAttribute("aria-checked", "false");
});

test("turned on mid-recording: asks first, then the badge appears", async ({ page }) => {
  await record(page);
  await page.getByRole("button", { name: "Record call", exact: true }).click();
  await live(page);
  await expect(page.getByTestId("sensitive-badge")).toHaveCount(0);

  await page.getByRole("button", { name: "More actions" }).click();
  await page.getByRole("menuitem", { name: "Sensitive meeting…" }).click();
  const ask = page.getByTestId("sensitive-confirm");
  await expect(ask.getByRole("alertdialog")).toContainText("This can't be turned off during this recording.");
  // Not a modal, and Cancel changes nothing.
  await ask.getByRole("button", { name: "Cancel" }).click();
  await expect(ask).toHaveCount(0);
  await expect(page.getByTestId("sensitive-badge")).toHaveCount(0);

  await page.getByRole("button", { name: "More actions" }).click();
  await page.getByRole("menuitem", { name: "Sensitive meeting…" }).click();
  await page.getByTestId("sensitive-confirm").getByRole("button", { name: "Make sensitive" }).click();
  await expect(page.getByTestId("sensitive-badge")).toBeVisible();
  await expect(page.getByTestId("sensitive-confirm")).toHaveCount(0);
});

test("a stored meeting: asks first, then no audio, no playback, no cloud", async ({ page }) => {
  await page.goto("/?platform=win#/meetings");
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await expect(page).toHaveURL(/#\/meetings\/sample-\d+\/notes/);
  await expect(page.getByTestId("sensitive-badge")).toHaveCount(0);
  await expect(page.getByTestId("audio-bar")).toBeVisible();

  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("menuitem", { name: "Sensitive meeting" }).click();
  const ask = page.getByTestId("sensitive-confirm");
  await expect(ask.getByRole("alertdialog")).toContainText("Its audio is deleted now and only the transcript stays.");
  await ask.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByTestId("sensitive-badge")).toHaveCount(0);
  await expect(page.getByTestId("audio-bar")).toBeVisible();

  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("menuitem", { name: "Sensitive meeting" }).click();
  await page.getByTestId("sensitive-confirm").getByRole("button", { name: "Make sensitive" }).click();
  await expect(page.getByTestId("sensitive-badge")).toHaveText("Sensitive · no audio kept");
  await expect(page.getByTestId("audio-bar")).toHaveCount(0);

  // The library row says so too, in words.
  await page.getByRole("button", { name: "Meetings", exact: true }).click();
  await expect(page.getByTestId("row-sensitive").first()).toHaveText("Sensitive");
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();

  // Off again: the flag only; the audio stays deleted.
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("menuitem", { name: "Sensitive meeting" }).click();
  await expect(page.getByTestId("sensitive-badge")).toHaveCount(0);
  await expect(page.getByTestId("audio-bar")).toHaveCount(0);
});
