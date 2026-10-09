// SPDX-License-Identifier: Apache-2.0
// The Map tab: the notes as a mind map on the mocked core (pan/zoom, collapse,
// playing a leaf, Show in transcript, the keyboard tree, copy as outline, axe,
// reduced motion and a visual baseline per theme and language).
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

// The mock core plays a blob: WAV, which the production media-src (ghi-audio: only) refuses.
test.use({ bypassCSP: true });

async function openMap(page: Page, query = "platform=win") {
  await page.goto(`/?${query}#/meetings`);
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await page.getByRole("tab", { name: "Map" }).click();
  await expect(page).toHaveURL(/\/map$/);
  await expect(page.getByTestId("mind-map")).toBeVisible();
}
const playerMs = (page: Page) => page.getByTestId("meeting-detail").getAttribute("data-player-ms").then(Number);
const node = (page: Page, text: string | RegExp) => page.locator("[data-node]").filter({ hasText: text }).first();
const section = (page: Page, id: string) => page.locator(`[data-node="sec:${id}"]`);
const tree = (page: Page) => page.getByRole("tree", { name: "Mind map outline" });

test("the map is the notes: sections, a Proposed chip, and a future kind under Other", async ({ page }) => {
  await openMap(page);
  for (const s of ["summary", "decisions", "proposed", "actions", "questions", "topics", "other"]) await expect(section(page, s)).toBeAttached();
  await expect(node(page, /Schedule a beta review/)).toContainText("Proposed");
  await expect(node(page, /A block of a newer kind/)).toBeAttached();
  // 3 sources on the first decision: the map has just one leaf for it.
  await expect(page.locator("[data-node]").filter({ hasText: /Live speaker rename ships in the beta/ })).toHaveCount(1);
});

test("the map stars items that cover a mark and lists the uncovered one", async ({ page }) => {
  await openMap(page);
  await expect(section(page, "marked")).toBeAttached();
  await expect(page.locator("[data-node]").filter({ hasText: /Live speaker rename ships/ })).toHaveCount(1);
  await expect(tree(page).getByRole("treeitem", { name: /Live speaker rename ships.*Covers a moment you marked/ })).toHaveCount(1);
});

test("a leaf click plays its first citation; Ctrl-click shows it in the transcript", async ({ page }) => {
  await openMap(page);
  const before = await playerMs(page);
  await node(page, /Live speaker rename ships/).click();
  await expect.poll(() => playerMs(page)).not.toBe(before);
  await node(page, /Voice profiles are saved only after/).click({ modifiers: ["ControlOrMeta"] });
  await expect(page).toHaveURL(/\/transcript\?t=\d+$/);
  await expect(page.locator("[data-seg][data-pulse=true]")).toHaveCount(1);
});

test("a section collapses and expands on click", async ({ page }) => {
  await openMap(page);
  const leaf = node(page, /Live speaker rename ships/);
  await expect(leaf).toBeAttached();
  await section(page, "decisions").click();
  await expect(leaf).toHaveCount(0);
  await section(page, "decisions").click();
  await expect(leaf).toBeAttached();
});

test("pan, zoom and fit move the canvas", async ({ page }) => {
  await openMap(page);
  const canvas = page.getByTestId("mind-map-canvas");
  const t0 = await canvas.evaluate((el) => (el as HTMLElement).style.transform);
  await page.getByRole("button", { name: "Zoom in" }).click();
  const t1 = await canvas.evaluate((el) => (el as HTMLElement).style.transform);
  expect(t1).not.toBe(t0);
  const box = (await page.getByTestId("mind-map").boundingBox())!;
  await page.mouse.move(box.x + 40, box.y + 40);
  await page.mouse.down();
  await page.mouse.move(box.x + 140, box.y + 90, { steps: 4 });
  await page.mouse.up();
  const t2 = await canvas.evaluate((el) => (el as HTMLElement).style.transform);
  expect(t2).not.toBe(t1);
  await page.getByRole("button", { name: "Fit", exact: true }).click();
  await expect.poll(() => canvas.evaluate((el) => (el as HTMLElement).style.transform)).toBe(t0);
});

test("a keyboard-only user walks the tree and plays a leaf", async ({ page }) => {
  await openMap(page);
  const decisions = tree(page).getByRole("treeitem", { name: "Decisions" });
  await decisions.focus();
  await expect(tree(page)).toBeVisible(); // the list shows while it has focus
  await page.keyboard.press("ArrowRight");
  await expect(tree(page).getByRole("treeitem", { name: /^Live speaker rename ships/ })).toBeFocused();
  await expect(node(page, /Live speaker rename ships/)).toHaveAttribute("data-current", "true");
  const before = await playerMs(page);
  await page.keyboard.press("Enter");
  await expect.poll(() => playerMs(page)).not.toBe(before);
  await page.keyboard.press("ArrowLeft");
  await expect(decisions).toBeFocused();
  await page.keyboard.press("ArrowLeft");
  await expect(decisions).toHaveAttribute("aria-expanded", "false");
});

test("Copy as outline puts the notes on the clipboard as a nested list", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "clipboard", { value: { writeText: (s: string) => ((window as unknown as { __copied: string }).__copied = s) && Promise.resolve() }, configurable: true });
  });
  await openMap(page);
  await page.getByRole("button", { name: "Copy as outline" }).click();
  await expect(page.getByText("Outline copied", { exact: true })).toBeVisible();
  const md = await page.evaluate(() => (window as unknown as { __copied: string }).__copied);
  expect(md).toMatch(/^# Client call/);
  expect(md).toContain("- **Decisions**\n  - Live speaker rename ships in the beta.");
  expect(md).toContain("- **Proposed**\n  - Schedule a beta review with the client.");
  expect(md).toMatch(/- \*\*Action items\*\*\n {2}- \[[ x]\] .+/);
});

test("reduced motion: view changes are not animated, otherwise they are", async ({ page }) => {
  await openMap(page);
  await page.getByRole("button", { name: "Zoom in" }).click();
  await expect(page.getByTestId("mind-map-canvas")).toHaveClass(/transition-transform/);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.getByRole("button", { name: "Zoom out" }).click();
  await expect(page.getByTestId("mind-map-canvas")).not.toHaveClass(/transition-transform/);
});

test("the tab row wraps with both arrows (Notes <- Map, Notes -> Transcript -> Map)", async ({ page }) => {
  await openMap(page);
  await page.getByRole("tab", { name: "Map" }).focus();
  await page.keyboard.press("ArrowLeft");
  await expect(page.getByRole("tab", { name: "Transcript" })).toBeFocused();
  await expect(page).toHaveURL(/\/transcript$/);
  await page.keyboard.press("ArrowLeft");
  await expect(page.getByRole("tab", { name: "Notes" })).toBeFocused();
  await page.keyboard.press("ArrowLeft");
  await expect(page.getByRole("tab", { name: "Map" })).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await expect(page.getByRole("tab", { name: "Notes" })).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await expect(page.getByRole("tab", { name: "Transcript" })).toBeFocused();
});

test("arrow keys on the map's first Tab stop walk into the tree", async ({ page }) => {
  await openMap(page);
  await page.getByTestId("mind-map").focus();
  await page.keyboard.press("ArrowDown");
  await expect(tree(page).getByRole("treeitem", { name: "Summary" })).toBeFocused();
});

test("axe finds nothing on the map, with the keyboard list open", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await openMap(page);
  const scan = async () => {
    const r = await new AxeBuilder({ page }).include('[data-testid="mind-map"]').withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
    expect(r.violations.map((v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
  };
  await scan();
  await tree(page).getByRole("treeitem", { name: "Summary" }).focus();
  await scan();
});

for (const lang of ["en", "vi"] as const)
  for (const theme of ["light", "dark"] as const)
    test(`visual baseline ${lang} ${theme}`, async ({ page, browserName }) => {
      test.skip(browserName !== "webkit" || process.platform !== "darwin", "WKWebView on macOS is what ships; CI has no baselines");
      await page.setViewportSize({ width: 1280, height: 800 });
      await page.addInitScript((state) => localStorage.setItem("ghira.prefs", JSON.stringify({ state, version: 0 })), { theme, language: lang });
      await openMapAny(page);
      await expect(page.getByTestId("mind-map")).toHaveScreenshot(`mind-map-${lang}-${theme}.png`, { animations: "disabled" });
    });

async function openMapAny(page: Page) {
  await page.goto("/?platform=mac#/meetings");
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await page.locator("#tab-map").click();
  await expect(page.getByTestId("mind-map")).toBeVisible();
  await page.waitForTimeout(300);
}
