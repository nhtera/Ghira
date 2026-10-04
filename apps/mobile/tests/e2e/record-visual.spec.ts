// SPDX-License-Identifier: Apache-2.0
// M2/M6 visual baselines: light/dark x EN/VI, and 200% text with Vietnamese
// diacritics (ệ, ỗ, ẫ) that must not clip. Update with --update-snapshots.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible } from "./helpers";
import { openRecord, startRecording } from "./record-support";

const LOOKS = [
  { id: "en-light", lang: "en", scheme: "light", scale: undefined },
  { id: "en-dark", lang: "en", scheme: "dark", scale: undefined },
  { id: "vi-light", lang: "vi", scheme: "light", scale: undefined },
  { id: "vi-dark", lang: "vi", scheme: "dark", scale: undefined },
  { id: "en-200", lang: "en", scheme: "light", scale: 2 },
  { id: "vi-200", lang: "vi", scheme: "light", scale: 2 },
] as const;

const phase = (page: Page, p: string) => page.evaluate((x) => window.__ghiMock!.simulateMobileEvent({ type: "phase", phase: x as never }), p);
const mobile = (page: Page, e: object) => page.evaluate((x) => window.__ghiMock!.simulateMobileEvent(x as never), e);

/** A frozen clock keeps the timer at 0:00 in every shot; the first speaker is Me (a voice profile matched). */
async function open(page: Page, look: (typeof LOOKS)[number], knobs: Record<string, unknown> = { firstIsMe: true }) {
  await page.clock.install();
  await page.emulateMedia({ colorScheme: look.scheme, reducedMotion: "reduce" });
  await openRecord(page, { lang: look.lang, scale: look.scale, knobs });
}

async function goLive(page: Page, n = 6) {
  await startRecording(page);
  await expect(page.getByRole("button", { name: /^(Stop|Dừng)$/ })).toBeVisible();
  await expect(page.getByText(/Getting ready|Đang chuẩn bị/)).toHaveCount(0);
  await page.evaluate((x) => window.__ghiRecord!.addLines(x), n);
  await expect(page.getByTestId("line")).not.toHaveCount(0);
}

for (const look of LOOKS) {
  test(`record idle ${look.id}`, async ({ page }) => {
    await open(page, look);
    await expect(page).toHaveScreenshot(`record-idle-${look.id}.png`, { animations: "disabled" });
  });

  test(`record live ${look.id}`, async ({ page }) => {
    await open(page, look);
    await goLive(page);
    await expect(page).toHaveScreenshot(`record-live-${look.id}.png`, { animations: "disabled" });
  });

  test(`record paused ${look.id}`, async ({ page }) => {
    await open(page, look);
    await goLive(page);
    await page.getByRole("button", { name: /^(Pause|Tạm dừng)$/ }).click();
    await expect(page.getByRole("button", { name: /^(Resume|Tiếp tục)$/ })).toBeVisible();
    await expect(page).toHaveScreenshot(`record-paused-${look.id}.png`, { animations: "disabled" });
  });

  test(`record catching up ${look.id}`, async ({ page }) => {
    await open(page, look);
    await goLive(page, 3);
    await mobile(page, { type: "backlog", backlogS: 60, catchUpX: 2 });
    await phase(page, "catchingUp");
    await mobile(page, { type: "backlog", backlogS: 20, catchUpX: 2 });
    await mobile(page, { type: "pocket", muffled: true });
    await expect(page.getByText(/%/).first()).toBeVisible();
    await expect(page).toHaveScreenshot(`record-catching-up-${look.id}.png`, { animations: "disabled" });
  });

  test(`call interruption sheet ${look.id}`, async ({ page }) => {
    await open(page, look);
    await goLive(page, 3);
    await mobile(page, { type: "interruption", began: true, kind: "call" });
    await phase(page, "interrupted");
    await expect(page.getByRole("dialog")).toBeVisible();
    await expect(page).toHaveScreenshot(`record-interrupted-${look.id}.png`, { animations: "disabled" });
  });

  test(`call notice sheet (M6) ${look.id}`, async ({ page }) => {
    await open(page, look, { callActive: true });
    await page.getByRole("button", { name: /record room|ghi phòng họp/i }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await expect(page).toHaveScreenshot(`record-call-notice-${look.id}.png`, { animations: "disabled" });
  });
}

/** Pause/Stop/Mark (or Record) fully inside 390x844, above the tab bar, nothing of the screen cut off. */
async function inside(page: Page, names: string[]) {
  const bar = (await page.getByRole("navigation").boundingBox())!;
  const spill = await page.evaluate(() => {
    const el = document.querySelector("[data-screen=record]")!;
    return el.scrollHeight - el.clientHeight;
  });
  expect(spill, "screen overflows").toBeLessThanOrEqual(0);
  for (const name of names) {
    const box = await page.getByRole("button", { name, exact: true }).boundingBox();
    expect(box, name).not.toBeNull();
    expect(box!.x, name).toBeGreaterThanOrEqual(0);
    expect(box!.y, name).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width, name).toBeLessThanOrEqual(390);
    expect(box!.y + box!.height, name).toBeLessThanOrEqual(Math.min(844, bar.y));
    expect(box!.height, name).toBeGreaterThanOrEqual(43.5);
  }
}

test("200% Vietnamese: diacritics stay whole, nothing clips or scrolls sideways", async ({ page }) => {
  await open(page, LOOKS[5]);
  await goLive(page, 6);
  // The sample lines carry ệ, ỗ, ẫ.
  await expect(page.getByText(/Ệ, ỗ, ẫ/)).toBeVisible();

  const report = await page.evaluate(() => {
    const clipped = [...document.querySelectorAll<HTMLElement>("[data-testid=line], [data-testid=line] p, [data-testid=line] b, h1")].filter((el) => el.scrollHeight > el.clientHeight + 1 || el.scrollWidth > el.clientWidth + 1).map((el) => el.textContent?.slice(0, 30));
    return { clipped, sideways: document.documentElement.scrollWidth - window.innerWidth };
  });
  expect(report.clipped).toEqual([]);
  expect(report.sideways).toBeLessThanOrEqual(0);

  // The thumb-zone controls are pinned, also with the busiest banners up. The transcript gives way.
  const controls = ["Dừng", "Tạm dừng", "Đánh dấu"];
  await inside(page, controls);
  await mobile(page, { type: "backlog", backlogS: 60, catchUpX: 2 });
  await phase(page, "catchingUp");
  await mobile(page, { type: "pocket", muffled: true });
  await expect(page.getByText(/Đang bắt kịp/)).toBeVisible();
  await inside(page, controls);
  await expectAccessible(page);
});

test("200% Vietnamese: idle, saved and record-only keep the record button in reach", async ({ page }) => {
  await open(page, LOOKS[5]);
  await inside(page, ["Ghi phòng họp"]);

  // Saved (and a start error) join the capped banner region.
  await goLive(page, 2);
  await page.getByRole("button", { name: "Dừng", exact: true }).click();
  await expect(page.getByText("Đã lưu bản ghi âm")).toBeVisible();
  await inside(page, ["Ghi phòng họp"]);
  await page.evaluate(() => (window.__ghiRecord!.failStart = "diskLow"));
  await page.getByRole("button", { name: "Ghi phòng họp" }).click();
  await page.getByRole("button", { name: /bắt đầu ghi âm/ }).click();
  await expect(page.getByText(/Không đủ dung lượng/)).toBeVisible();
  await inside(page, ["Ghi phòng họp"]);
  await expectAccessible(page);
});

test("200% Vietnamese: a record-only phone, idle and recording", async ({ page }) => {
  await page.clock.install();
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await openRecord(page, { lang: "vi", scale: 2, knobs: { tier: "recordOnly" } });
  await expect(page.getByText(/Chỉ ghi âm trên điện thoại này/)).toBeVisible();
  await inside(page, ["Ghi phòng họp"]);
  await startRecording(page);
  await expect(page.getByRole("button", { name: "Dừng", exact: true })).toBeVisible();
  await inside(page, ["Dừng", "Tạm dừng", "Đánh dấu"]);
  await expectAccessible(page);
});
