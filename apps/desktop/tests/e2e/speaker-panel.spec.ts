// SPDX-License-Identifier: Apache-2.0
// Design 7c, the speaker side panel of a stored meeting, on the mocked core:
// open from a transcript line, rename, Me, merge (with its question), split
// from a line, not a person, the core's refusals in words, axe, and a visual
// baseline per theme.
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

async function open(page: Page, title: RegExp, query = "platform=win") {
  await page.goto(`/?${query}#/meetings`);
  await page.getByRole("button", { name: title }).click();
  await page.getByRole("tab", { name: "Transcript" }).click();
  await expect(page.getByTestId("transcript-group").first()).toBeVisible();
}
const room = (page: Page, query?: string) => open(page, /1:1 với Linh/, query);
const call = (page: Page) => open(page, /Client call — Acme onboarding/);
const panel = (page: Page) => page.getByTestId("speaker-panel");
const nameButton = (page: Page, name: string) => page.getByRole("button", { name: `Speaker options for ${name}` });

test("opens from a speaker's name on a line, is a labelled dialog, and Escape returns focus", async ({ page }) => {
  await room(page);
  const opener = nameButton(page, "Sarah").first();
  await opener.click();
  await expect(panel(page)).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Sarah" })).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Rename speaker" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(panel(page)).toHaveCount(0);
  await expect(opener).toBeFocused();
});

test("Tab stays inside the panel", async ({ page }) => {
  await room(page);
  await nameButton(page, "Sarah").first().click();
  for (let i = 0; i < 14; i++) {
    await page.keyboard.press("Tab");
    expect(await panel(page).evaluate((el) => el.contains(document.activeElement))).toBe(true);
  }
});

test("rename applies to every line of the speaker in the transcript", async ({ page }) => {
  await room(page);
  await nameButton(page, "Sarah").first().click();
  const box = page.getByRole("textbox", { name: "Rename speaker" });
  await box.fill("Sara Lee");
  await box.press("Enter");
  await expect(page.getByText("Renamed to Sara Lee on every line", { exact: true })).toBeVisible();
  await expect(nameButton(page, "Sara Lee").first()).toBeVisible();
  await expect(nameButton(page, "Sarah")).toHaveCount(0);
});

test("mark as Me and back; a call has no Me buttons", async ({ page }) => {
  await room(page);
  await nameButton(page, "Sarah").first().click();
  await panel(page).getByRole("button", { name: "This is me" }).click();
  await expect(page.getByText("Marked as Me", { exact: true })).toBeVisible();
  await expect(panel(page).getByRole("button", { name: "Not me" })).toBeVisible();
  await page.keyboard.press("Escape");
  await call(page);
  await nameButton(page, "Sarah").first().click();
  await expect(panel(page)).toBeVisible();
  await expect(panel(page).getByRole("button", { name: "This is me" })).toHaveCount(0);
});

test("merge asks first; Cancel keeps both, Merge moves the lines and closes the panel", async ({ page }) => {
  await room(page);
  await nameButton(page, "Sarah").first().click();
  await panel(page).getByRole("button", { name: /Minh/ }).click();
  const ask = panel(page).getByRole("alertdialog");
  await expect(ask).toContainText(/Merge Sarah into Minh\? All \d+ lines move to Minh\. There is no undo\./);
  await expect(ask.getByRole("button", { name: "Cancel" })).toBeFocused();
  await ask.getByRole("button", { name: "Cancel" }).click();
  await expect(nameButton(page, "Sarah").first()).toBeVisible();
  await panel(page).getByRole("button", { name: /Minh/ }).click();
  await ask.getByRole("button", { name: "Merge" }).click();
  await expect(page.getByText("Merged into Minh", { exact: true })).toBeVisible();
  await expect(panel(page)).toHaveCount(0);
  await expect(nameButton(page, "Sarah")).toHaveCount(0);
});

test("a call refuses merging Me into a far-side speaker, in words", async ({ page }) => {
  await call(page);
  await nameButton(page, "An Tran").first().click();
  await panel(page).getByRole("button", { name: /Sarah/ }).click();
  await panel(page).getByRole("alertdialog").getByRole("button", { name: "Merge" }).click();
  await expect(page.getByTestId("speaker-panel-error")).toHaveText("In a call only your own microphone can be Me, so this speaker can’t be merged with Me.");
  await expect(panel(page)).toBeVisible();
});

test("split from this line on creates a new speaker", async ({ page }) => {
  await room(page);
  const groups = page.getByTestId("transcript-group").filter({ has: nameButton(page, "Sarah") });
  const before = await groups.count();
  // From the last paragraph of Sarah's, so that earlier lines stay with her.
  await groups.last().getByRole("button", { name: "Speaker options for Sarah" }).click();
  await panel(page).getByRole("button", { name: "Split speaker…" }).click();
  await expect(panel(page).getByRole("radio", { name: /^From \d\d:\d\d on/ })).toBeChecked();
  await panel(page).getByRole("button", { name: /^Move \d+ lines?$/ }).click();
  await expect(page.getByText(/^Moved \d+ lines? to a new speaker$/)).toBeVisible();
  await expect(groups).toHaveCount(before - 1);
  await expect(page.getByTestId("transcript-group").getByRole("button", { name: /Speaker options for Speaker \d/ }).first()).toBeVisible();
});

test("split these lines: pick by checkbox; moving every line is not offered", async ({ page }) => {
  await room(page);
  await nameButton(page, "Sarah").first().click();
  await panel(page).getByRole("button", { name: "Split speaker…" }).click();
  await panel(page).getByRole("radio", { name: "These lines" }).check();
  const boxes = panel(page).getByRole("checkbox");
  const n = await boxes.count();
  expect(n).toBeGreaterThan(1);
  await expect(panel(page).getByRole("button", { name: /^Move 0 lines$/ })).toBeDisabled();
  for (let i = 0; i < n; i++) await boxes.nth(i).check();
  await expect(panel(page).getByRole("button", { name: `Move ${n} lines` })).toBeDisabled();
  await boxes.nth(0).uncheck();
  await panel(page).getByRole("button", { name: `Move ${n - 1} ${n - 1 === 1 ? "line" : "lines"}` }).click();
  await expect(page.getByText(/^Moved \d+ lines? to a new speaker$/)).toBeVisible();
});

test("not a person asks first, hides them from the notes, and can be undone from the panel", async ({ page }) => {
  await room(page);
  await nameButton(page, "Sarah").first().click();
  await panel(page).getByRole("button", { name: "Not a person (video, music)" }).click();
  await expect(panel(page).getByRole("alertdialog")).toContainText("Mark Sarah as not a person?");
  await panel(page).getByRole("button", { name: "Mark as not a person" }).click();
  await expect(page.getByText(/marked as not a person/)).toBeVisible();
  await expect(panel(page).getByRole("button", { name: "This is a person" })).toBeVisible();
  await panel(page).getByRole("button", { name: "This is a person" }).click();
  await expect(panel(page).getByRole("button", { name: "Not a person (video, music)" })).toBeVisible();
});

test("Me cannot be marked as not a person: the panel says so", async ({ page }) => {
  await room(page);
  await nameButton(page, "An Tran").first().click();
  await panel(page).getByRole("button", { name: "Not a person (video, music)" }).click();
  await panel(page).getByRole("button", { name: "Mark as not a person" }).click();
  await expect(page.getByTestId("speaker-panel-error")).toHaveText("Me can’t be marked as not a person. Mark someone else as Me first.");
});

test("switching to another speaker starts clean: nothing typed for the first is saved to the second", async ({ page }) => {
  await room(page);
  await nameButton(page, "Sarah").first().click();
  await page.getByRole("textbox", { name: "Rename speaker" }).fill("Typed for Sarah");
  await panel(page).getByRole("button", { name: /Minh/ }).click();
  await expect(panel(page).getByRole("alertdialog")).toBeVisible();
  // The page behind is still clickable: pick another speaker's name.
  await nameButton(page, "Linh").first().click({ force: true });
  await expect(page.getByRole("dialog", { name: "Linh" })).toBeVisible();
  const box = page.getByRole("textbox", { name: "Rename speaker" });
  await expect(box).toHaveValue("Linh");
  await expect(panel(page).getByRole("alertdialog")).toHaveCount(0);
  await expect(panel(page).getByRole("button", { name: "Save" })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(nameButton(page, "Sarah").first()).toBeVisible();
  await expect(nameButton(page, "Typed for Sarah")).toHaveCount(0);
});

test("a click on the panel's own text keeps Escape and the Tab trap working", async ({ page }) => {
  await room(page);
  await nameButton(page, "Sarah").first().click();
  await panel(page).getByText("Speaker · Speaker 2").click();
  await page.keyboard.press("Tab");
  expect(await panel(page).evaluate((el) => el.contains(document.activeElement) && document.activeElement !== el)).toBe(true);
  await panel(page).getByText("Speaker · Speaker 2").click();
  await page.keyboard.press("Escape");
  await expect(panel(page)).toHaveCount(0);
});

test("Shift+Tab from the first control wraps to the last", async ({ page }) => {
  await room(page);
  await nameButton(page, "Sarah").first().click();
  await panel(page).getByRole("button", { name: "Close" }).focus();
  await page.keyboard.press("Shift+Tab");
  expect(await panel(page).evaluate((el) => el.contains(document.activeElement))).toBe(true);
  await expect(panel(page).getByRole("button", { name: "Not a person (video, music)" })).toBeFocused();
});

test("axe finds nothing on the open panel", async ({ page }) => {
  await room(page);
  await nameButton(page, "Sarah").first().click();
  await panel(page).getByRole("button", { name: "Split speaker…" }).click();
  const r = await new AxeBuilder({ page }).include("[data-testid=speaker-panel]").withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
  expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(" ")}`)).toEqual([]);
});

for (const theme of ["light", "dark"] as const)
  test(`visual baseline ${theme}`, async ({ page, browserName }) => {
    test.skip(browserName !== "webkit" || process.platform !== "darwin", "WKWebView on macOS is what ships; CI has no baselines");
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.addInitScript((state) => localStorage.setItem("ghira.prefs", JSON.stringify({ state, version: 0 })), { theme, language: "en" });
    await room(page, "platform=mac");
    await nameButton(page, "Sarah").first().click();
    await expect(panel(page)).toBeVisible();
    await expect(page).toHaveScreenshot(`speaker-panel-${theme}.png`, { animations: "disabled", mask: [page.getByTestId("audio-bar")] });
  });
