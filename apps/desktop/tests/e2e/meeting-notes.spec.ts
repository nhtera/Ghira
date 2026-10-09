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
  await expect(first).toBeVisible();
  const all = await notes.count();
  await first.click();
  await first.press("End");
  await first.pressSequentially(" (checked)");
  await page.getByRole("heading", { name: "Summary" }).click();
  await expect(page.getByText("Edited by you · kept on regenerate").first()).toBeVisible();
  await page.getByRole("switch", { name: "My notes only" }).click();
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

test("Transcribe again asks for the spoken language, then processes the meeting", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("menuitem", { name: "Transcribe again…" }).click();
  const panel = page.getByRole("alertdialog", { name: /Transcribe the recording again/ });
  await expect(panel.getByRole("radio", { name: "English", exact: true })).toHaveAttribute("aria-checked", "true");
  await expect(panel.getByRole("button", { name: "Cancel" })).toBeFocused();
  await panel.getByRole("radio", { name: "Tiếng Việt" }).click();
  await panel.getByRole("button", { name: "Transcribe again" }).click();
  await expect(panel).toBeHidden();
  await expect(page.getByText("Transcribing again…", { exact: true })).toBeVisible();
});

test("a decision with three sources is one chip with +2; the preview steps and shows the line in the transcript", async ({ page }) => {
  await open(page);
  const more = page.getByRole("button", { name: "2 more sources" });
  await expect(more).toHaveText("+2");
  await expect(page.getByRole("button", { name: "2 more sources" })).toHaveCount(1);
  // By keyboard: Safari does not focus a button on click, and the arrows act on the focused chip.
  await more.focus();
  await page.keyboard.press("Enter");
  const preview = page.getByRole("group", { name: "Quote preview" });
  await expect(preview).toContainText("2/3");
  await page.keyboard.press("ArrowRight");
  await expect(preview).toContainText("3/3");
  await page.keyboard.press("ArrowRight");
  await expect(preview).toContainText("1/3");
  await preview.getByRole("button", { name: "Next source" }).click();
  await preview.getByRole("button", { name: "Show in transcript" }).click();
  await expect(page).toHaveURL(/\/transcript\?t=\d+$/);
  await expect(page.getByTestId("transcript-group").first()).toBeVisible();
  // The landing line pulses (or is outlined under reduced motion), then settles.
  await expect(page.locator("[data-seg][data-pulse=true]")).toHaveCount(1);
  await expect(page.locator("[data-seg][data-pulse=true]")).toHaveCount(0, { timeout: 3000 });
});

test("items that cover a marked moment carry a star; the uncovered mark is listed to play", async ({ page }) => {
  await open(page);
  const stars = page.getByTestId("mark-star");
  await expect(stars.first()).toBeVisible();
  await expect(stars.first()).toHaveAttribute("title", /^You marked this at \d\d:\d\d( · You marked this at \d\d:\d\d)*$/);
  const moments = page.getByTestId("marked-moments");
  await expect(page.getByRole("heading", { name: "Moments you marked" })).toBeVisible();
  await expect(moments.getByRole("listitem")).toHaveCount(1);
  await expect(moments).toContainText("Question");
  const before = await playerMs(page);
  await moments.getByRole("button", { name: /^Play from/ }).click();
  await expect.poll(() => playerMs(page)).not.toBe(before);
  // The waveform has a tick for each of the three marks.
  await expect(page.getByTestId("waveform")).toHaveAttribute("data-marks", "3");
});

test("Decisions lists decided items, then Proposed ones with a chip and the footnote; the block menu moves one and keeps focus", async ({ page }) => {
  await open(page);
  const decisions = page.getByRole("region", { name: "Decisions" });
  await expect(decisions.getByTestId("proposed-chip")).toHaveCount(1);
  await expect(decisions.getByTestId("proposal-footnote")).toHaveText("AI suggestions are not commitments.");
  // decided ones come first
  const order = await decisions.locator("textarea").evaluateAll((els) => els.map((e) => (e as HTMLTextAreaElement).value));
  expect(order.at(-1)).toBe("Schedule a beta review with the client.");
  // Decided -> Proposed
  const first = decisions.locator("[data-block]").first();
  const gid = await first.getAttribute("data-block");
  const menu = decisions.locator(`[data-block="${gid}"]`).getByRole("button", { name: "Decision options" });
  await menu.click();
  await page.getByRole("menuitem", { name: "Mark as proposed" }).click();
  await expect(decisions.getByTestId("proposed-chip")).toHaveCount(2);
  await expect(decisions.locator(`[data-block="${gid}"]`).getByTestId("proposed-chip")).toBeVisible();
  // keyboard focus is back on that block's menu button, wherever the block went
  await expect(menu).toBeFocused();
  // and back
  await menu.click();
  await page.getByRole("menuitem", { name: "Mark as decided" }).click();
  await expect(decisions.getByTestId("proposed-chip")).toHaveCount(1);
  await expect(menu).toBeFocused();
});
