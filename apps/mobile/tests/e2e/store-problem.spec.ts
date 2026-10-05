// SPDX-License-Identifier: Apache-2.0
// "Ghira can't open your meetings on this phone": the screen for a store that
// can't be opened (one case per Rust error code), Try again, Start fresh with
// the typed phrase, and the "couldn't load" states that keep any failed start
// from being a blank page.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible } from "./helpers";

const CODES = ["keyMissing", "keyLocked", "keystore", "damaged", "migration", "disk", "other"] as const;
const REASON: Record<(typeof CODES)[number], RegExp> = {
  keyMissing: /kept on this phone only/,
  keyLocked: /Unlock with Face ID or your passcode/,
  keystore: /secure key storage/,
  damaged: /may be damaged/,
  migration: /couldn’t be updated/,
  disk: /free space/,
  other: /Something went wrong/,
};
const problemDialog = (page: Page) => page.getByRole("dialog", { name: /can’t open your meetings/ });

async function launch(page: Page, query: string, route = "/meetings") {
  await page.goto(`/?${query}#${route}`);
  await page.waitForFunction(() => Boolean(window.__ghiMock));
}

for (const code of CODES) {
  test(`${code}: the reason in words, the code, and a way forward`, async ({ page }) => {
    await launch(page, `storeProblem=${code}`);
    const dialog = problemDialog(page);
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(REASON[code]);
    await expect(page.getByTestId("store-problem-code")).toHaveText(`Error code: ${code}`);
    await expect(dialog.getByRole("button", { name: "Try again" })).toBeVisible();
    // Only data that is gone for good may be erased.
    await expect(dialog.getByRole("button", { name: "Start fresh" })).toHaveCount(code === "keyMissing" || code === "damaged" ? 1 : 0);
    // The page behind is out of reach.
    await expect(page.locator("#root")).toHaveJSProperty("inert", true);
    await expectAccessible(page);
  });
}

test("Try again says so while it still fails, and carries on once it opens", async ({ page }) => {
  await launch(page, "storeProblem=keyMissing");
  const dialog = problemDialog(page);
  await dialog.getByRole("button", { name: "Try again" }).click();
  await expect(dialog.getByRole("alert")).toContainText("still can’t be opened");
  await page.evaluate(() => (window.__ghiSettingsMock!.storeProblem = null));
  await dialog.getByRole("button", { name: "Try again" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.locator("#root")).toHaveJSProperty("inert", false);
  await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
});

test("Start fresh needs the typed phrase, then restarts at onboarding", async ({ page }) => {
  await launch(page, "storeProblem=damaged");
  await expect(problemDialog(page)).toBeVisible();
  // The dialog is named by its heading, which changes with the step.
  const dialog = page.getByTestId("store-problem-gate");
  await dialog.getByRole("button", { name: "Start fresh" }).click();
  await expect(dialog.getByRole("heading", { name: "Start fresh?" })).toBeFocused();
  const erase = dialog.getByRole("button", { name: "Erase and start fresh" });
  await expect(erase).toBeDisabled();
  await dialog.getByLabel("Type DELETE to confirm").fill("delet");
  await expect(erase).toBeDisabled();
  await expectAccessible(page);

  // Cancel goes back; nothing was wiped.
  await dialog.getByRole("button", { name: "Cancel" }).click();
  expect(await page.evaluate(() => window.__ghiSettingsMock!.wiped)).toBe(false);
  await dialog.getByRole("button", { name: "Start fresh" }).click();

  await dialog.getByLabel("Type DELETE to confirm").fill("delete");
  await erase.click();
  await expect(dialog).toHaveCount(0);
  expect(await page.evaluate(() => window.__ghiSettingsMock!.wiped)).toBe(true);
  await expect(page).toHaveURL(/#\/onboarding/);
  await expect(page.getByRole("heading", { name: /languages/ })).toBeVisible();
});

test("Vietnamese: the words and the XOÁ phrase", async ({ page }) => {
  await launch(page, "lang=vi&storeProblem=keyMissing");
  await expect(page.getByRole("dialog", { name: /không mở được/ })).toContainText("Chuỗi khoá");
  const dialog = page.getByTestId("store-problem-gate");
  await dialog.getByRole("button", { name: "Bắt đầu lại từ đầu" }).click();
  await dialog.getByLabel("Gõ XOÁ để xác nhận").fill("xoá");
  await expect(dialog.getByRole("button", { name: "Xoá và bắt đầu lại" })).toBeEnabled();
});

test("a start that keeps failing shows couldn't load with Try again, not a blank page", async ({ page }) => {
  await launch(page, "startupFails=1");
  const dialog = page.getByRole("dialog", { name: /couldn’t load/ });
  await expect(dialog).toBeVisible({ timeout: 15_000 });
  await expect(dialog.getByRole("button", { name: "Try again" })).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Start fresh" })).toHaveCount(0);
  await expectAccessible(page);
  await page.evaluate(() => (window.__ghiSettingsMock!.startupFails = false));
  await dialog.getByRole("button", { name: "Try again" }).click();
  await expect(dialog).toHaveCount(0);
});

test("a store that opened but failed to start shows couldn't load, with no erase", async ({ page }) => {
  await launch(page, "storeProblem=startup");
  const dialog = page.getByRole("dialog", { name: /couldn’t load/ });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Start fresh" })).toHaveCount(0);
});

test("a route that doesn't exist says so and goes to Meetings", async ({ page }) => {
  await launch(page, "", "/no/such/screen");
  const notFound = page.getByTestId("page-not-found");
  await expect(notFound).toBeVisible();
  await expect(notFound).toContainText("Page not found");
  await expect(notFound.getByRole("button", { name: "Try again" })).toHaveCount(0);
  await expectAccessible(page);
  await notFound.getByRole("button", { name: "Go to Meetings" }).click();
  await expect(page.getByRole("heading", { name: "Meetings", level: 1 })).toBeVisible();
});
