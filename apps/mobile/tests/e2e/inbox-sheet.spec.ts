// SPDX-License-Identifier: Apache-2.0
// M5 inside the app (16-J): files the share extension left behind wait for a
// language and target; importing shows a toast; nothing shows while locked.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible, openApp } from "./helpers";

const item = (id: string, over: Record<string, unknown> = {}) => ({ id, name: `${id}.m4a`, sizeBytes: 12_000_000, language: "auto", target: "phone", state: "pending", reason: null, ...over });
const seed = (page: Page, items: unknown[]) => page.evaluate((i) => window.__ghiSettingsMock!.setInbox(i as never), items);

test.beforeEach(async ({ page }) => {
  await openApp(page, "/settings");
});

test("waiting files raise a banner; Review opens the choices", async ({ page }) => {
  await seed(page, [item("standup"), item("review")]);
  const banner = page.getByRole("status").filter({ hasText: "2 files are waiting to import" });
  await expect(banner).toBeVisible();
  await banner.getByRole("button", { name: "Review" }).click();
  const dialog = page.getByRole("dialog", { name: "Waiting to import" });
  await expect(dialog.getByText("standup.m4a")).toBeVisible();
  await expect(dialog.getByRole("button", { name: "My computer" }).first()).toBeDisabled();
  await expectAccessible(page);
});

test("the banner can be hidden; a new waiting file brings it back", async ({ page }) => {
  await seed(page, [item("standup"), item("review")]);
  const banner = page.getByRole("status").filter({ hasText: "files are waiting to import" });
  await expect(banner).toBeVisible();
  const hide = banner.getByRole("button", { name: "Hide notice" });
  expect((await hide.boundingBox())!.width).toBeGreaterThanOrEqual(44);
  await hide.click();
  await expect(banner).toHaveCount(0);
  // Same files, still waiting: it stays hidden.
  await seed(page, [item("standup"), item("review")]);
  await expect(banner).toHaveCount(0);
  // One more arrives: back.
  await seed(page, [item("standup"), item("review"), item("retro")]);
  await expect(page.getByRole("status").filter({ hasText: "3 files are waiting to import" })).toBeVisible();
});

test("Settings lists the waiting files after the banner is hidden, and opens the same sheet", async ({ page }) => {
  // None waiting: the row says so and does nothing.
  await expect(page.getByText("None", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: /Waiting to import/ })).toHaveCount(0);
  await seed(page, [item("standup"), item("review")]);
  await page.getByRole("status").filter({ hasText: "files are waiting to import" }).getByRole("button", { name: "Hide notice" }).click();
  await expect(page.getByRole("status").filter({ hasText: "files are waiting to import" })).toHaveCount(0);
  const row = page.getByRole("button", { name: /Waiting to import/ });
  await expect(row).toContainText("2");
  await row.click();
  const dialog = page.getByRole("dialog", { name: "Waiting to import" });
  await expect(dialog.getByText("standup.m4a")).toBeVisible();
  await expect(dialog.getByText("review.m4a")).toBeVisible();
  // One can be imported from here.
  await dialog.getByRole("listitem").filter({ hasText: "standup.m4a" }).getByRole("button", { name: "Import" }).click();
  await expect(dialog.getByText("standup.m4a")).toHaveCount(0);
});

test("the banner sits above the screen's content, not over it (iPhone safe areas)", async ({ page }) => {
  // A notched iPhone: 59 pt of status bar, 34 pt of home indicator.
  await page.evaluate(() => {
    document.documentElement.style.setProperty("--safe-top", "59px");
    document.documentElement.style.setProperty("--safe-bottom", "34px");
  });
  await seed(page, [item("standup"), item("review")]);
  const banner = page.getByRole("status").filter({ hasText: "files are waiting to import" });
  await expect(banner).toBeVisible();
  for (const [route, content] of [
    ["#/record", () => page.getByRole("heading", { level: 1 })],
    ["#/record", () => page.getByRole("timer")],
    ["#/meetings", () => page.getByRole("heading", { level: 1, name: "Meetings" })],
    ["#/search", () => page.getByRole("heading", { level: 1, name: "Search" })],
  ] as const) {
    await page.evaluate((r) => (window.location.hash = r), route);
    await expect(content()).toBeVisible();
    const b = (await banner.boundingBox())!;
    const c = (await content().boundingBox())!;
    expect(b.y, "below the status bar").toBeGreaterThanOrEqual(59);
    expect(c.y, `${route} content starts under the banner`).toBeGreaterThanOrEqual(b.y + b.height);
  }
  // The strip wears the screen's background: no seam on any tab, light or dark.
  for (const dark of [false, true]) {
    await page.emulateMedia({ colorScheme: dark ? "dark" : "light" });
    for (const [route, screen] of [
      ["#/settings", "settings"],
      ["#/record", "record"],
      ["#/meetings", "meetings"],
    ] as const) {
      await page.evaluate((r) => (window.location.hash = r), route);
      await expect(page.locator(`[data-notices]`)).toBeVisible();
      const [slot, page_] = await page.evaluate((name) => {
        const bg = (el: Element | null) => (el ? getComputedStyle(el).backgroundColor : "");
        const screenEl = document.querySelector(`[data-screen=${name}]`) ?? document.querySelector("main > *");
        const own = bg(screenEl);
        // A screen with no background of its own shows the page's.
        const shown = own === "rgba(0, 0, 0, 0)" ? bg(document.body) : own;
        return [bg(document.querySelector("[data-notices]")), shown];
      }, screen);
      expect(slot, `${screen} ${dark ? "dark" : "light"}`).toBe(page_);
    }
  }
  await page.emulateMedia({ colorScheme: "light" });
  // Hidden: the screen gets its own top inset back.
  await banner.getByRole("button", { name: "Hide notice" }).click();
  await expect(banner).toHaveCount(0);
  await page.evaluate(() => (window.location.hash = "#/record"));
  expect((await page.getByRole("heading", { level: 1 }).boundingBox())!.y).toBeGreaterThanOrEqual(59);
});

test("confirming imports the file, shows the toast, and nothing else was imported", async ({ page }) => {
  await seed(page, [item("standup"), item("review")]);
  await page.getByRole("button", { name: "Review" }).click();
  const dialog = page.getByRole("dialog", { name: "Waiting to import" });
  const first = dialog.getByRole("listitem").filter({ hasText: "standup.m4a" });
  await first.getByRole("button", { name: "VI" }).click();
  await first.getByRole("button", { name: "Import" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Added to Ghira" })).toBeVisible();
  await expect(dialog.getByText("standup.m4a")).toHaveCount(0);
  expect(await page.evaluate(() => window.__ghiSettingsMock!.args.inboxConfirm?.slice(1))).toEqual(["vi", "phone"]);
  expect(await page.evaluate(() => window.__ghiSettingsMock!.calls.inboxConfirm)).toBe(1);
  await expect(dialog.getByText("review.m4a")).toBeVisible();
});

test("the last file imported closes the sheet and the toast shows on its own", async ({ page }) => {
  await seed(page, [item("only")]);
  await page.getByRole("button", { name: "Review" }).click();
  const dialog = page.getByRole("dialog", { name: "Waiting to import" });
  await dialog.getByRole("button", { name: "Import" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: "Added to Ghira" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Review" })).toHaveCount(0);
});

test("a rejected file explains why and can be removed", async ({ page }) => {
  // The banner counts files that wait for a choice, so a pending one opens the sheet.
  await seed(page, [item("voice-memo", { name: "voice-memo.caf", state: "rejected", reason: "unsupportedType" }), item("b")]);
  await page.getByRole("button", { name: "Review" }).click();
  const dialog = page.getByRole("dialog", { name: "Waiting to import" });
  await expect(dialog.getByRole("alert")).toContainText("This file type isn’t supported.");
  await dialog.getByRole("button", { name: "Remove voice-memo.caf" }).click();
  await expect(dialog.getByText("voice-memo.caf")).toHaveCount(0);
});

test("a busy core keeps the file and says why", async ({ page }) => {
  await seed(page, [item("a")]);
  await page.evaluate(() => (window.__ghiSettingsMock!.busy = true));
  await page.getByRole("button", { name: "Review" }).click();
  const dialog = page.getByRole("dialog", { name: "Waiting to import" });
  await dialog.getByRole("button", { name: "Import" }).click();
  await expect(dialog.getByRole("alert")).toContainText("A recording or import is running");
  await expect(dialog.getByText("a.m4a")).toBeVisible();
});

test("while the app is locked the inbox stays hidden, then shows after unlock", async ({ page }) => {
  await page.evaluate(() => {
    window.__ghiSettingsMock!.faceIdOk = false;
    window.__ghiSettingsMock!.locked = true;
    window.dispatchEvent(new Event("focus"));
  });
  await seed(page, [item("a")]);
  await expect(page.getByRole("button", { name: "Review" })).toHaveCount(0);
  await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = true));
  await page.getByRole("button", { name: "Unlock with Face ID" }).click();
  await expect(page.getByRole("button", { name: "Review" })).toBeVisible();
});

test("locking clears the inbox from the screen, unlocking brings it back", async ({ page }) => {
  await seed(page, [item("a")]);
  await expect(page.getByRole("button", { name: "Review" })).toBeVisible();
  await page.getByRole("button", { name: "Review" }).click();
  await page.evaluate(() => {
    window.__ghiSettingsMock!.faceIdOk = false;
    window.__ghiSettingsMock!.locked = true;
    window.dispatchEvent(new Event("focus"));
  });
  await expect(page.getByRole("dialog", { name: "Waiting to import" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Review" })).toHaveCount(0);
  await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = true));
  await page.getByRole("button", { name: "Unlock with Face ID" }).click();
  await expect(page.getByRole("button", { name: "Review" })).toBeVisible();
});

test("a hidden banner stays hidden across a lock and unlock with the same files", async ({ page }) => {
  await seed(page, [item("a"), item("b")]);
  await page.getByRole("status").filter({ hasText: "files are waiting to import" }).getByRole("button", { name: "Hide notice" }).click();
  await page.evaluate(() => {
    window.__ghiSettingsMock!.faceIdOk = false;
    window.__ghiSettingsMock!.locked = true;
    window.dispatchEvent(new Event("focus"));
  });
  await expect(page.getByRole("button", { name: "Unlock with Face ID" })).toBeVisible();
  await page.evaluate(() => (window.__ghiSettingsMock!.faceIdOk = true));
  await page.getByRole("button", { name: "Unlock with Face ID" }).click();
  // The files are back (Settings counts them) but the banner is not.
  await expect(page.getByRole("button", { name: /Waiting to import/ })).toContainText("2");
  await expect(page.getByRole("button", { name: "Review" })).toHaveCount(0);
  // One more still brings it back.
  await seed(page, [item("a"), item("b"), item("c")]);
  await expect(page.getByRole("button", { name: "Review" })).toBeVisible();
});

test("the Added toast is an overlay: it is not in the notice strip and does not sit at the top", async ({ page }) => {
  await seed(page, [item("only")]);
  await page.getByRole("button", { name: "Review" }).click();
  await page.getByRole("dialog", { name: "Waiting to import" }).getByRole("button", { name: "Import" }).click();
  const toast = page.getByRole("status").filter({ hasText: "Added to Ghira" });
  await expect(toast).toBeVisible();
  await expect(page.locator("[data-notices]")).toHaveCount(1);
  await expect(page.locator("[data-notices]")).toBeHidden();
  expect((await toast.boundingBox())!.y).toBeGreaterThan(page.viewportSize()!.height / 2);
});
