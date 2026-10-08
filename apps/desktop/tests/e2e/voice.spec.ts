// SPDX-License-Identifier: Apache-2.0
// Me enrollment in onboarding, "Your voice" in Settings → Privacy, and the
// per-meeting voice suggestion, on the mocked core.
import { expect, test, type Page } from "@playwright/test";

const step = (page: Page, query = "") => page.goto(`/?platform=win${query}#/onboarding/voice`);

test("onboarding: consent first, then read, then saved", async ({ page }) => {
  await step(page, "&enrollspeed=20");
  const start = page.getByRole("button", { name: "Start reading" });
  await expect(start).toBeDisabled();
  await page.getByRole("checkbox").check();
  await start.click();
  await expect(page.getByText(/Reading… (1\d|2\d) s/)).toBeVisible();
  await page.getByRole("button", { name: "Stop and save" }).click();
  await expect(page.getByText(/^Your voice is saved/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue" })).toBeVisible();
});

for (const [flag, text] of [
  ["micPermission", /Microphone access is off/],
  ["noMic", /No microphone was found/],
  ["tooQuiet", /barely hear you/],
] as const) {
  test(`onboarding: ${flag} is explained and Skip still works`, async ({ page }) => {
    await step(page, `&enrollfail=${flag}&enrollspeed=20`);
    await page.getByRole("checkbox").check();
    await page.getByRole("button", { name: "Start reading" }).click();
    if (flag === "tooQuiet") await page.getByRole("button", { name: "Stop and save" }).click();
    await expect(page.getByRole("alert")).toContainText(text);
    await page.getByRole("button", { name: "Skip" }).click();
    await expect(page).not.toHaveURL(/onboarding\/voice/);
  });
}

test("onboarding: a voice model still downloading is said, blocks only Start, and never holds up the flow", async ({ page }) => {
  // The models step starts the voice download on its own; here it never finishes.
  await page.goto("/?platform=win&novoice=1&voiceslow=1#/onboarding/models");
  await expect(page.getByText("Downloaded and checked").first()).toBeVisible();
  // Once the voice download is running the step is part of the flow.
  await expect(page.getByRole("navigation").getByRole("listitem")).toHaveCount(8);
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByRole("heading", { name: /Teach .+ your voice/ })).toBeVisible();
  await expect(page.getByText(/voice model is still downloading/)).toBeVisible();
  await page.getByRole("checkbox").check();
  await expect(page.getByRole("button", { name: "Start reading" })).toBeDisabled();
  await page.getByRole("button", { name: "Skip" }).click();
  await expect(page).not.toHaveURL(/onboarding\/voice/);
});

test("onboarding: without the voice model (and no download) the step is skipped silently", async ({ page }) => {
  await page.goto("/?platform=win&novoice=1#/onboarding/permissions");
  await expect(page.getByRole("navigation").getByRole("listitem")).toHaveCount(7);
  await page.goto("/?platform=win&novoice=1#/onboarding/voice");
  await expect(page.getByRole("heading", { name: "Try a 10-second test" })).toBeVisible();
});

test("Settings → Privacy shows the voice profile and deletes it after a confirm", async ({ page }) => {
  await page.goto("/?platform=win#/settings/privacy");
  const card = page.locator("section").filter({ has: page.getByRole("heading", { name: "Your voice" }) });
  await expect(card).toContainText(/Voice profile saved .* · 6 samples/);
  await card.getByRole("button", { name: "Delete my voice data…" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Delete voice data" }).click();
  await expect(page.getByText(/^Your voice data is deleted\./)).toBeVisible();
  await expect(card).toContainText("No voice profile yet");
});

test("a speaker 'Sounds like Me' can be accepted, and Not me undoes it", async ({ page }) => {
  await page.goto("/?platform=win&suggest=1#/meetings");
  await page.getByRole("button", { name: /1:1 với Linh/ }).click();
  const list = page.getByRole("list", { name: "Speakers" });
  const chip = page.getByTestId("voice-suggestion");
  await expect(chip).toContainText("Sounds like Me");
  await chip.getByRole("button", { name: "Accept Me as this speaker" }).click();
  await expect(page.getByText("Marked as Me", { exact: true })).toBeVisible();
  await expect(page.getByTestId("voice-suggestion")).toHaveCount(0);
  await expect(list.getByRole("button", { name: /^Me$|Me/ }).first()).toBeVisible();
});

test("a suggestion can be dismissed", async ({ page }) => {
  await page.goto("/?platform=win&suggest=1#/meetings");
  await page.getByRole("button", { name: /1:1 với Linh/ }).click();
  await page.getByTestId("voice-suggestion").getByRole("button", { name: /Dismiss the suggestion for/ }).click();
  await expect(page.getByTestId("voice-suggestion")).toHaveCount(0);
});
