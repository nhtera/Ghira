// SPDX-License-Identifier: Apache-2.0
// The live meeting view on the mocked core: notepad, tags, pause, discard,
// capture conditions, layouts, long transcripts, the compact window.
// Copy still pending in PENDING-ui-live.json shows as its key until the
// locale files are updated, so those controls are found by role or test id.
import { expect, test, type Page } from "@playwright/test";

type Mock = { __ghiMock: { simulateCoreEvent: (e: object) => void } };
const emit = (page: Page, event: object) => page.evaluate((e) => (window as unknown as Mock).__ghiMock.simulateCoreEvent({ meeting: "", ...e }), event);

async function record(page: Page) {
  await page.goto("/?platform=win#/meetings");
  await page.getByRole("button", { name: "Record call", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "Meeting title" })).toBeVisible();
  await expect(page.getByRole("main").locator("ol > li").first()).toContainText("Okay, bắt đầu nhé.", { timeout: 5000 });
}
const pad = (page: Page) => page.getByRole("region", { name: "Your notes" });
const padInput = (page: Page) => pad(page).locator("input").last();

test("notepad: type, tag, edit, delete", async ({ page }) => {
  await record(page);
  await padInput(page).fill("ship the beta");
  await padInput(page).press("Enter");
  await expect(pad(page).getByRole("listitem").filter({ hasText: "ship the beta" })).toBeVisible();

  await padInput(page).fill("go with option B");
  await pad(page).getByRole("button", { name: "Decision" }).click();
  const decision = pad(page).locator("li[data-kind=decision]");
  await expect(decision).toContainText("go with option B");
  // A tagged line is a mark for the notes.
  await expect(page.getByText(/^1 marked/)).toBeVisible();

  await padInput(page).fill("who owns this");
  await padInput(page).press("Alt+Digit3");
  await expect(pad(page).locator("li[data-kind=question]")).toContainText("who owns this");

  const first = pad(page).getByRole("listitem").filter({ hasText: "ship the beta" });
  await first.hover();
  await first.getByRole("button", { name: "Edit text" }).click();
  await page.getByRole("textbox", { name: "Edit text" }).fill("ship the beta on Friday");
  await page.keyboard.press("Enter");
  await expect(pad(page)).toContainText("ship the beta on Friday");
  await pad(page).getByRole("listitem").filter({ hasText: "on Friday" }).hover();
  await pad(page).getByRole("listitem").filter({ hasText: "on Friday" }).getByRole("button", { name: "Delete" }).click();
  await expect(pad(page)).not.toContainText("on Friday");
});

test("pause and resume show the paused state", async ({ page }) => {
  await record(page);
  await page.getByRole("button", { name: "Pause" }).click();
  // Paused is an overlay over the transcript and notes with Resume as its one action.
  await expect(page.getByTestId("paused-overlay")).toContainText("Paused. Nothing is being recorded.");
  await page.getByTestId("paused-overlay").getByRole("button", { name: "Resume" }).click();
  await expect(page.getByTestId("paused-overlay")).toHaveCount(0);
});

test("discard the last minutes: preview, then confirm", async ({ page }) => {
  await record(page);
  await page.locator("button[aria-haspopup=menu]").click();
  // The discard items follow the sensitive-mode item.
  await page.getByRole("menuitem", { name: /^Discard the last/ }).nth(1).click();
  const panel = page.getByTestId("discard-panel");
  await expect(panel.getByRole("alertdialog")).toBeVisible();
  // Not a modal: the page behind stays usable and nothing is removed yet.
  await expect(page.getByRole("main").locator("ol > li").first()).toContainText("Okay, bắt đầu nhé.");
  await panel.getByRole("button", { name: "Cancel" }).click();
  await expect(panel).toHaveCount(0);

  await page.locator("button[aria-haspopup=menu]").click();
  // The discard items follow the sensitive-mode item.
  await page.getByRole("menuitem", { name: /^Discard the last/ }).nth(1).click();
  await panel.getByRole("button").first().click();
  await expect(panel).toHaveCount(0);
  // The cut reaches back to the start on the mock: the lines are gone.
  await expect(page.getByRole("main").locator("ol > li").filter({ hasText: "Okay, bắt đầu nhé." })).toHaveCount(0);
});

test("capture conditions show as inline banners, not dialogs", async ({ page }) => {
  await record(page);
  const banner = (id: string) => page.locator(`[data-banner=${id}]`);
  await emit(page, { type: "slept" });
  await expect(banner("asleep")).toBeVisible();
  await emit(page, { type: "woke" });
  await expect(banner("asleep")).toHaveCount(0);
  await emit(page, { type: "silentSystemTrack", silentS: 12 });
  await expect(banner("system-silent")).toContainText("System audio access is off");
  await emit(page, { type: "trackLost", track: 0 });
  await expect(banner("mic-lost")).toBeVisible();
  await emit(page, { type: "diskLow", freeBytes: 480_000_000 });
  await expect(banner("disk-low")).toContainText("Only 480 MB left");
  await emit(page, { type: "diskFull" });
  await expect(banner("disk-full")).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("health opens into rows and suggests Fast mode when behind", async ({ page }) => {
  await record(page);
  await emit(page, { type: "health", asrLagS: 4.5, asrSkippedS: 0, aec: false });
  await page.getByTestId("health").getByRole("button").click();
  // The rows are in a popover (portaled), not inside the footer chip.
  const row = page.locator("[data-row=asr]");
  await expect(row).toHaveAttribute("data-warn", "true");
  await expect(page.getByRole("button", { name: "Switch to Fast mode" })).toBeVisible();
  await expect(page.locator("[data-row]")).toHaveCount(4);
});

test("consent confirmed is a per-meeting toggle", async ({ page }) => {
  await record(page);
  // The toggle lives in the More menu; once on, the header shows "Consent confirmed".
  const toggle = async () => {
    await page.getByRole("button", { name: "More actions" }).click();
    return page.getByRole("menuitemcheckbox", { name: "Consent confirmed" });
  };
  const off = await toggle();
  await expect(off).toHaveAttribute("aria-checked", "false");
  await expect(page.getByTestId("consent-confirmed")).toHaveCount(0);
  await off.click();
  await expect(page.getByTestId("consent-confirmed")).toBeVisible();
  const on = await toggle();
  await expect(on).toHaveAttribute("aria-checked", "true");
  await page.keyboard.press("Escape");
});

test("the copy-consent helper puts the message on the clipboard", async ({ page, context, browserName }) => {
  test.skip(browserName !== "chromium", "clipboard permissions are chromium-only");
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await record(page);
  await page.getByRole("button", { name: "Copy consent message" }).click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain("recording this meeting");
});

test("focus layout puts the notepad in front and back again", async ({ page }) => {
  await record(page);
  const left = async (loc: ReturnType<Page["locator"]>) => (await loc.boundingBox())!.x;
  await expect(page.getByTestId("transcript-scroll")).toBeVisible();
  // Transcript layout: the transcript is the wide column on the left, notes on the right.
  expect(await left(page.getByTestId("transcript-scroll"))).toBeLessThan(await left(pad(page)));
  await page.getByRole("radio", { name: "Focus" }).click();
  // Focus layout: the notepad moves to the front and the transcript shrinks beside it.
  await expect(pad(page)).toBeVisible();
  await expect(page.getByTestId("transcript-scroll")).toBeVisible();
  expect(await left(pad(page))).toBeLessThan(await left(page.getByTestId("transcript-scroll")));
  await page.getByRole("radio", { name: "Transcript" }).click();
  expect(await left(page.getByTestId("transcript-scroll"))).toBeLessThan(await left(pad(page)));
});

test("a long transcript is virtualized; scrolling up stops following and jump to live returns", async ({ page }) => {
  await record(page);
  await page.evaluate(() => {
    const emit = (window as unknown as Mock).__ghiMock.simulateCoreEvent;
    for (let i = 0; i < 400; i++) {
      emit({ type: "transcriptFinal", track: 0, meeting: "", line: { gid: `bulk-${i}`, speaker: 1, t0Ms: 100_000 + i * 2000, t1Ms: 101_500 + i * 2000, text: `bulk line number ${i}`, overlap: false, words: [] } });
    }
  });
  const rows = page.getByRole("main").locator("ol > li");
  await expect(page.getByTestId("transcript-scroll")).toBeVisible();
  expect(await rows.count()).toBeLessThan(80);
  await expect(page.getByTestId("jump-to-live")).toHaveCount(0);
  await page.getByTestId("transcript-scroll").evaluate((el) => {
    el.scrollTop = 0;
  });
  await expect(page.getByTestId("jump-to-live")).toBeVisible();
  await page.getByTestId("jump-to-live").click();
  await expect(page.getByTestId("jump-to-live")).toHaveCount(0);
  await expect(page.getByText("bulk line number 399")).toBeVisible();
});

test("more than eight speakers share an Others lane", async ({ page }) => {
  await record(page);
  await page.evaluate(() => {
    const emit = (window as unknown as Mock).__ghiMock.simulateCoreEvent;
    for (let id = 1; id <= 10; id++) {
      emit({ type: "speakerArrived", meeting: "", speaker: { id: 100 + id, label: `Speaker ${100 + id}`, colorSlot: ((id - 1) % 8) + 1, isMe: false, provisional: false, notPerson: false, others: false } });
    }
  });
  await page.getByRole("button", { name: "Timeline" }).click();
  await expect(page.locator("[data-lane]")).toHaveCount(9);
  await expect(page.locator('[data-lane="0"]')).toHaveCount(1);
});

test("compact window: the header fits and keeps accessible names", async ({ page }) => {
  await page.setViewportSize({ width: 960, height: 640 });
  await record(page);
  await expect(page.getByRole("button", { name: /Mark moment/ })).toBeVisible();
  await expect(page.getByRole("button", { name: "More actions" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});

test("more than eight voices: Others · 3 on the lane, a +3 chip, and the note once", async ({ page }) => {
  await record(page);
  // Eleven speakers arrive (the mock already has a few); 9 to 11 are past the palette.
  for (let id = 1; id <= 11; id++) {
    await emit(page, { type: "speakerArrived", speaker: { id, label: `Speaker ${id}`, colorSlot: id <= 8 ? id : 0, isMe: false, provisional: false, notPerson: false, others: id > 8 } });
  }
  await expect(page.getByTestId("others-chip")).toContainText("Others");
  await expect(page.getByTestId("others-chip")).toHaveAccessibleName("3 voices in Others");
  await page.getByRole("button", { name: "Timeline" }).click();
  await expect(page.getByText("Others · 3").first()).toBeVisible();
  await expect(page.locator("[data-banner=many-voices]")).toHaveCount(1);
  await expect(page.locator("[data-banner=many-voices]")).toContainText("More than 8 voices");
  await page.getByTestId("others-chip").click();
  await expect(page.getByRole("dialog", { name: "3 voices in Others" })).toContainText("Speaker 11");
  // A twelfth does not bring a second note.
  await page.keyboard.press("Escape");
  await emit(page, { type: "speakerArrived", speaker: { id: 12, label: "Speaker 12", colorSlot: 0, isMe: false, provisional: false, notPerson: false, others: true } });
  await expect(page.getByTestId("others-chip")).toHaveAccessibleName("4 voices in Others");
  await expect(page.locator("[data-banner=many-voices]")).toHaveCount(1);
});

test("a call stacks lines that overlap in time and marks them", async ({ page }) => {
  await record(page);
  const base = { type: "transcriptFinal", track: 0, line: { speaker: 1, text: "Chốt ngân sách trước thứ Sáu", overlap: true, words: [] } };
  await emit(page, { ...base, line: { ...base.line, gid: "ov-1", t0Ms: 600_000, t1Ms: 606_000 } });
  await emit(page, { ...base, line: { ...base.line, gid: "ov-2", speaker: 2, text: "Cho tôi nói với", t0Ms: 604_000, t1Ms: 609_000 } });
  const stack = page.getByTestId("transcript-stack");
  await expect(stack).toBeVisible();
  // The header says it all; each of the two lines keeps the short label.
  await expect(stack.getByTestId("overlap-tag")).toHaveCount(3);
  await expect(stack).toContainText("Cho tôi nói với");
});
