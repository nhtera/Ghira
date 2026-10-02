// SPDX-License-Identifier: Apache-2.0
// Transcript tab and audio bar of the meeting detail on the mocked core.
import { expect, test, type Page } from "@playwright/test";

// The mock core plays a blob: WAV, which the production media-src (ghi-audio: only) refuses.
test.use({ bypassCSP: true });

async function openTranscript(page: Page, platform = "win") {
  await page.goto(`/?platform=${platform}#/meetings`);
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await page.getByRole("tab", { name: "Transcript" }).click();
  await expect(page.getByTestId("transcript-group").first()).toBeVisible();
}

const playerMs = (page: Page) => page.getByTestId("meeting-detail").getAttribute("data-player-ms").then(Number);

test("clicking a line plays from it and the audio bar follows", async ({ page }) => {
  await openTranscript(page);
  await expect(page.getByTestId("audio-bar")).toBeVisible();
  const before = await playerMs(page);
  await page.getByTestId("transcript-group").nth(1).locator("p").first().click();
  await expect.poll(() => playerMs(page)).toBeGreaterThan(before);
  await expect(page.getByTestId("meeting-detail")).toHaveAttribute("data-player-playing", "true");
  // The playing line is marked.
  await expect(page.locator("[data-playing=true]").first()).toBeVisible();
});

test("find ignores accents: nhan dien finds nhận diện", async ({ page }) => {
  await openTranscript(page);
  await page.getByRole("searchbox", { name: "Find in transcript" }).fill("nhan dien");
  const marks = page.locator("mark");
  await expect(marks.first()).toBeVisible();
  await expect(marks.first()).toHaveText(/nhận/i);
});

test("Ctrl+F in the tab focuses find", async ({ page }) => {
  await openTranscript(page);
  await page.getByTestId("transcript-group").first().click();
  await page.keyboard.press("Control+f");
  await expect(page.getByRole("searchbox", { name: "Find in transcript" })).toBeFocused();
});

test("editing a line shows Edited", async ({ page }) => {
  await openTranscript(page);
  const line = page.getByTestId("transcript-group").first().locator("[data-seg]").first();
  await line.dblclick();
  const box = page.getByRole("textbox", { name: "Edit text" });
  await box.fill("Chào cả nhà, mình bắt đầu nhé");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(page.getByTestId("transcript-group").first().getByText("Edited")).toBeVisible();
  await expect(page.getByText("Chào cả nhà, mình bắt đầu nhé").first()).toBeVisible();
});

test("the waveform is a keyboard slider", async ({ page }) => {
  await openTranscript(page);
  const slider = page.getByRole("slider").first();
  await slider.focus();
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => playerMs(page)).toBe(5000);
  await page.keyboard.press("End");
  await expect.poll(() => playerMs(page)).toBeGreaterThan(2000 * 1000);
});
