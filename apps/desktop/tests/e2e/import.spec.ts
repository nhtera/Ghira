// SPDX-License-Identifier: Apache-2.0
// Import (D10) on the mocked core: choose files, problems inline, queue, done.
import { expect, test } from "@playwright/test";

test("choose files → problems shown → import → queue → done with Open notes", async ({ page }) => {
  await page.goto("/?platform=win#/import");
  await expect(page.getByText("Drop audio or video files here")).toBeVisible();
  await page.getByRole("button", { name: "Choose files…" }).first().click();
  const staged = page.getByRole("region", { name: "Ready to import" });
  await expect(staged).toBeVisible();
  // The sample list has a duplicate, a very long file and an unreadable one.
  await expect(staged.getByText(/Already imported as “Client call — Acme onboarding”/)).toBeVisible();
  await expect(staged.getByRole("button", { name: "Open", exact: true })).toBeVisible();
  await expect(staged.getByText(/Over 4 hours/)).toBeVisible();
  await expect(staged.getByText(/can’t read this file/)).toBeVisible();
  // A stereo file offers split channels.
  await expect(page.getByRole("checkbox", { name: /Split stereo channels/ })).toBeVisible();
  // Remove the blocked ones, then import the rest.
  await staged.getByRole("button", { name: /^Remove / }).last().click();
  const go = page.getByRole("button", { name: /^Import \d+ files?$/ });
  await expect(go).toBeEnabled();
  await go.click();
  const queue = page.getByRole("region", { name: "Queue" });
  await expect(queue.getByText(/files? importing/)).toBeVisible();
  await expect(queue.getByText(/Converting \d+%/).first()).toBeVisible({ timeout: 8000 });
  await expect(queue.getByRole("button", { name: "Open notes" }).first()).toBeVisible({ timeout: 20000 });
  await expect(page.getByText(/^\d+ files? imported\. Notes are ready\.$/)).toBeVisible({ timeout: 20000 });
});

test("files dragged over the window show the drop target", async ({ page }) => {
  await page.goto("/?platform=win#/import");
  // The screen listens once it has mounted.
  await expect(page.getByRole("button", { name: "Choose files…" })).toBeVisible();
  await page.evaluate(() => {
    const dt = new DataTransfer();
    dt.items.add(new File(["x"], "a.mp3"));
    window.dispatchEvent(new DragEvent("dragenter", { dataTransfer: dt }));
  });
  await expect(page.getByText("Drop to import into Ghira")).toBeVisible();
  await page.evaluate(() => window.dispatchEvent(new DragEvent("dragleave", { dataTransfer: new DataTransfer() })));
});

test("a drop staged by the core appears in the list", async ({ page }) => {
  await page.goto("/?platform=win#/import");
  await page.evaluate(() => (window as unknown as { __ghiMock: { simulateImportDrop: () => void } }).__ghiMock.simulateImportDrop());
  await expect(page.getByRole("region", { name: "Ready to import" })).toBeVisible();
});
