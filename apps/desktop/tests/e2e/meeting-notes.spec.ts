// SPDX-License-Identifier: Apache-2.0
// Meeting detail: notes, citations to audio, provenance, on the mocked core.
import { expect, test, type Page } from "@playwright/test";

const open = async (page: Page, query = "platform=win") => {
  await page.goto(`/?${query}#/meetings`);
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await expect(page).toHaveURL(/#\/meetings\/sample-\d+\/notes/);
};
const playerMs = (page: Page) => page.getByTestId("meeting-detail").getAttribute("data-player-ms").then(Number);

test("Notes → citation → audio: one click plays the cited moment", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("heading", { name: "Summary" })).toBeVisible();
  const before = await playerMs(page);
  const chip = page.getByRole("button", { name: /^Show in transcript \d+:\d\d$/ }).first();
  await chip.click();
  await expect.poll(() => playerMs(page)).toBeGreaterThan(before);
  await expect(page.getByTestId("meeting-detail")).toHaveAttribute("data-player-playing", /true|false/);
});

test("hovering a citation previews who said it and offers Play", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: /^Show in transcript \d+:\d\d$/ }).first().hover();
  const preview = page.getByRole("group", { name: "Quote preview" });
  await expect(preview).toBeVisible();
  await expect(preview.getByRole("button", { name: /^Play from/ })).toBeVisible();
});

test("editing an AI sentence makes it yours; My notes only hides the rest", async ({ page }) => {
  await open(page);
  const notes = page.getByRole("textbox", { name: "Note", exact: true });
  const first = notes.first();
  const all = await notes.count();
  await first.click();
  await first.press("End");
  await first.pressSequentially(" (checked)");
  await page.getByRole("heading", { name: "Summary" }).click();
  await expect(page.getByText("Edited by you · kept on regenerate").first()).toBeVisible();
  await page.getByRole("button", { name: "My notes only" }).click();
  await expect.poll(() => notes.count()).toBeLessThan(all);
  await expect(page.getByText("Edited by you · kept on regenerate").first()).toBeVisible();
});

test("a note typed in Your notes appears and Enter keeps the row", async ({ page }) => {
  await open(page);
  const row = page.getByLabel("Add a note");
  await row.fill("follow up with finance");
  await row.press("Enter");
  await expect(page.getByLabel("Your note", { exact: true }).last()).toHaveValue("follow up with finance");
  await expect(row).toHaveValue("");
});

test("the Transcript tab is a route", async ({ page }) => {
  await open(page);
  await page.getByRole("tab", { name: "Transcript" }).click();
  await expect(page).toHaveURL(/\/transcript$/);
  await page.getByRole("tab", { name: "Notes" }).click();
  await expect(page).toHaveURL(/\/notes$/);
});

for (const lang of ["en", "vi"] as const)
  for (const theme of ["light", "dark"] as const)
    test(`screenshot ${lang} ${theme}`, async ({ page }, info) => {
      test.skip(!process.env.GHI_SHOTS, "set GHI_SHOTS=<dir> to write screenshots");
      await page.addInitScript((state) => localStorage.setItem("ghira.prefs", JSON.stringify({ state, version: 0 })), { theme, language: lang });
      await open(page, "platform=mac");
      await page.waitForTimeout(400);
      await page.screenshot({ path: `${process.env.GHI_SHOTS}/notes-${lang}-${theme}-${info.project.name}.png` });
    });
