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
  await page.getByRole("button", { name: "New recording" }).click();
  await expect(page.getByRole("heading", { name: "Live" })).toBeVisible();
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
  await expect(page.getByText("1 marked")).toBeVisible();

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
  await expect(page.locator("[data-banner=paused]")).toContainText("Paused. Nothing is being recorded.");
  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.locator("[data-banner=paused]")).toHaveCount(0);
});

test("discard the last minutes: preview, then confirm", async ({ page }) => {
  await record(page);
  await page.locator("button[aria-haspopup=menu]").click();
  await page.getByRole("menuitem").nth(1).click();
  const panel = page.getByTestId("discard-panel");
  await expect(panel.getByRole("alertdialog")).toBeVisible();
  // Not a modal: the page behind stays usable and nothing is removed yet.
  await expect(page.getByRole("main").locator("ol > li").first()).toContainText("Okay, bắt đầu nhé.");
  await panel.getByRole("button", { name: "Cancel" }).click();
  await expect(panel).toHaveCount(0);

  await page.locator("button[aria-haspopup=menu]").click();
  await page.getByRole("menuitem").nth(1).click();
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
  const row = page.getByTestId("health").locator("[data-row=asr]");
  await expect(row).toHaveAttribute("data-warn", "true");
  await expect(row).toContainText("Switch to Fast mode");
  await expect(page.getByTestId("health").locator("[data-row]")).toHaveCount(4);
});

test("consent confirmed is a per-meeting toggle", async ({ page }) => {
  await record(page);
  const toggle = page.getByRole("switch");
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
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
  await expect(page.getByTestId("transcript-scroll")).toBeVisible();
  await page.getByRole("radio", { name: "Focus" }).click();
  await expect(page.getByTestId("transcript-scroll")).toHaveCount(0);
  await expect(page.getByTestId("focus-caption")).toBeVisible();
  await expect(pad(page)).toBeVisible();
  await page.getByRole("radio", { name: "Transcript" }).click();
  await expect(page.getByTestId("transcript-scroll")).toBeVisible();
});

test("a long transcript is virtualized; scrolling up stops following and jump to live returns", async ({ page }) => {
  await record(page);
  await page.evaluate(() => {
    const emit = (window as unknown as Mock).__ghiMock.simulateCoreEvent;
    for (let i = 0; i < 400; i++) {
      emit({ type: "transcriptFinal", meeting: "", line: { gid: `bulk-${i}`, speaker: 1, t0Ms: 100_000 + i * 2000, t1Ms: 101_500 + i * 2000, text: `bulk line number ${i}`, overlap: false, words: [] } });
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
  await expect(page.locator("[data-lane]")).toHaveCount(9);
  await expect(page.locator('[data-lane="0"]')).toHaveCount(1);
});

test("compact window: the header fits and keeps accessible names", async ({ page }) => {
  await page.setViewportSize({ width: 960, height: 640 });
  await record(page);
  await expect(page.getByRole("button", { name: /Mark moment/ })).toBeVisible();
  await expect(page.getByRole("button", { name: "Consent confirmed" }).or(page.getByRole("switch"))).toBeVisible();
  await expect(page.getByRole("button", { name: "Stop" })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});
