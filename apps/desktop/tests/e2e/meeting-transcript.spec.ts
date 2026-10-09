// SPDX-License-Identifier: Apache-2.0
// Transcript tab and audio bar of the meeting detail on the mocked core.
import AxeBuilder from "@axe-core/playwright";
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
  // A seek before the audio's metadata loads is ignored by the element (a slow
  // machine gets the keys in first).
  await page.waitForFunction(() => (document.querySelector("audio")?.readyState ?? 0) >= 1);
  const slider = page.getByRole("slider").first();
  await slider.focus();
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => playerMs(page)).toBe(5000);
  await page.keyboard.press("End");
  await expect.poll(() => playerMs(page)).toBeGreaterThan(2000 * 1000);
});

test("a talked-over line shows the marker with its hint; a call stacks the two lines in one bracket", async ({ page }) => {
  await page.goto("/?platform=win&overlap=1#/meetings");
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await page.getByRole("tab", { name: "Transcript" }).click();
  // The transcript is virtualized: find takes the view to the first flagged line.
  await page.getByRole("searchbox", { name: "Find in transcript" }).fill("agreed");
  const stack = page.getByTestId("transcript-stack");
  await expect(stack).toBeVisible();
  await expect(stack).toHaveAttribute("aria-label", "Talking over each other");
  await expect(stack.getByTestId("transcript-group")).toHaveCount(2);
  const tags = stack.getByTestId("overlap-tag");
  // The header carries the hint; the two lines only the short label.
  await expect(tags).toHaveCount(3);
  await expect(tags.first()).toHaveAttribute("title", "Two people spoke at once here, so some words may be wrong.");
  await expect(tags.nth(1)).not.toHaveAttribute("title", /.+/);
  // Lines inside the bracket are still lines: they play from a click.
  const before = await playerMs(page);
  await stack.getByTestId("transcript-group").nth(1).locator("p").first().click();
  await expect.poll(() => playerMs(page)).not.toBe(before);
});

test("without overlap in the data there is no marker and no bracket", async ({ page }) => {
  await openTranscript(page);
  await expect(page.getByTestId("transcript-stack")).toHaveCount(0);
  await expect(page.getByTestId("overlap-tag")).toHaveCount(0);
});

test("the talk share bar sums to 100% and a legend entry opens the speaker panel", async ({ page }) => {
  await openTranscript(page);
  const share = page.getByTestId("talk-share");
  await expect(share).toContainText(/\d+ speakers · \d+ turns/);
  const pcts = (await share.locator("li").allTextContents()).map((t) => Number(/(\d+)%/.exec(t)?.[1]));
  expect(pcts.reduce((a, b) => a + b, 0)).toBe(100);
  await share.getByRole("button").first().click();
  await expect(page.getByTestId("speaker-panel")).toBeVisible();
});

test("a long meeting shows the topic rail wide and the collapsible Outline row when narrow", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await openTranscript(page);
  await expect(page.getByTestId("topic-rail")).toBeVisible();
  await expect(page.getByTestId("topic-outline")).toBeHidden();
  await page.setViewportSize({ width: 1000, height: 800 });
  await expect(page.getByTestId("topic-rail")).toBeHidden();
  const toggle = page.getByRole("button", { name: /^Outline \(\d+\)$/ });
  await expect(toggle).toBeVisible();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await toggle.click();
  const before = await playerMs(page);
  await page.getByTestId("topic-outline").getByRole("button").nth(3).click();
  await expect.poll(() => playerMs(page)).not.toBe(before);
});

test("axe: talk share bar, Outline row (narrow) and an open citation preview with the stepper", async ({ page }) => {
  const scan = async (include: string) => {
    const r = await new AxeBuilder({ page }).include(include).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
    expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
  };
  await page.setViewportSize({ width: 1000, height: 800 });
  await openTranscript(page);
  await scan('[data-testid="talk-share"]');
  await page.getByRole("button", { name: /^Outline \(\d+\)$/ }).click();
  await scan('[data-testid="topic-outline"]');
  await page.getByRole("tab", { name: "Notes" }).click();
  const more = page.getByRole("button", { name: "2 more sources" });
  await more.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("group", { name: "Quote preview" })).toContainText("2/3");
  await scan('[role="group"][aria-label="Quote preview"]');
});

