// SPDX-License-Identifier: Apache-2.0
// The app lock (D12) on the mocked core: `?locked=1` opens locked.
import { expect, test } from "@playwright/test";

test("a locked app shows only the lock screen until unlocked", async ({ page }) => {
  await page.goto("/?locked=1#/meetings");
  const lock = page.getByRole("alertdialog", { name: "Ghira is locked" });
  await expect(lock).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Main" })).toHaveCount(0);
  await lock.getByRole("button", { name: /Unlock/ }).click();
  await expect(page.getByRole("navigation", { name: "Main" })).toBeVisible();
});
