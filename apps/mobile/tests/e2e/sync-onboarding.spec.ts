// SPDX-License-Identifier: Apache-2.0
// 15-I: the Pair step of M1 on the scripted mock: scan, paired, not now, and the
// ways a scan fails (with Personal Hotspot guidance when the computer is out of reach).
import { expect, test } from "@playwright/test";
import { expectAccessible } from "./helpers";
import { mockHook, openPairStep, openSync } from "./sync-support";

const step = (page: import("@playwright/test").Page) => page.locator("[data-screen=onboarding]");

test("pairing is a step before processing: scan, Paired with <name>, Continue", async ({ page }) => {
  await openPairStep(page);
  await expect(step(page)).toHaveAttribute("data-step", "pair");
  await expect(page.getByRole("heading", { name: "Pair with your computer" })).toBeVisible();
  await expect(page.getByText("Step 4 of 8")).toBeVisible();
  await expect(page.getByTestId("pair-viewfinder")).toHaveAttribute("data-phase", "scanning");
  await expect(page.getByText("looking for a code")).toBeVisible();
  await expect(page.getByText(/Both devices need to be on the same Wi-Fi/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Not now" })).toBeVisible();
  await expectAccessible(page);

  await expect(page.getByTestId("pair-viewfinder")).toHaveAttribute("data-phase", "scanning");
  await mockHook(page, "syncSimulateScan");
  const card = page.getByTestId("paired-card");
  await expect(card).toContainText("Paired with MacBook Pro");
  // Voice profiles do not sync in v1: the copy must not say they do.
  await expect(card).toContainText("Voice profiles stay on each device");
  await expect(page.getByRole("button", { name: "Not now" })).toHaveCount(0);
  await expectAccessible(page);

  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(step(page)).toHaveAttribute("data-step", "processing");
  // Paired: My computer can be the default; the hint names it.
  const mine = page.getByRole("radio", { name: "My computer" });
  await expect(mine).toBeEnabled();
  await mine.check({ force: true });
  await expect(mine).toBeChecked();
  await expect(page.getByTestId("need-pair")).toHaveCount(0);
});

test("Not now skips pairing; the processing step then says to pair first", async ({ page }) => {
  await openPairStep(page);
  await page.getByRole("button", { name: "Not now" }).click();
  await expect(step(page)).toHaveAttribute("data-step", "processing");
  await expect(page.getByRole("radio", { name: "My computer" })).toBeDisabled();
  await expect(page.getByTestId("need-pair")).toHaveText("Pair a computer first");
  // Back to the pair step: nothing lost, the scan starts again.
  await page.getByRole("button", { name: "Back" }).click();
  await expect(page.getByTestId("pair-viewfinder")).toHaveAttribute("data-phase", "scanning");
});

test("resumes at the pair step when the steps before it are done", async ({ page }) => {
  await openPairStep(page, { completed: ["languages", "micPriming", "consent"] });
  await expect(step(page)).toHaveAttribute("data-step", "pair");
  await expect(page.getByTestId("onboarding-progress").locator("span")).toHaveCount(8);
});

for (const [code, message, settings] of [
  ["invalid", "That isn’t a Ghira code.", false],
  ["expired", "That code has expired. Show a new one on your computer.", false],
  ["cameraOff", "The camera is off for Ghira. Turn it on in Settings to scan the code.", true],
  ["localNetwork", "Ghira needs the local network to find your computer. Turn it on in Settings.", true],
] as const) {
  test(`a scan that fails with ${code} says why and can be retried`, async ({ page }) => {
    await openPairStep(page, { before: () => mockHook(page, "syncFailNextScan", code) });
    await expect(page.getByRole("status").filter({ hasText: message })).toBeVisible();
    await expect(page.getByTestId("hotspot-help")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Open Settings" })).toHaveCount(settings ? 1 : 0);
    await expectAccessible(page);
    await page.getByRole("button", { name: "Try again" }).click();
    await expect(page.getByTestId("pair-viewfinder")).toHaveAttribute("data-phase", "scanning");
    await mockHook(page, "syncSimulateScan");
    await expect(page.getByTestId("paired-card")).toBeVisible();
  });
}

test("not on the same Wi-Fi: guides to Personal Hotspot, then the export", async ({ page }) => {
  await openPairStep(page);
  // The mock only fails a running scan: wait for the camera to start (a slow
  // machine gets the hook in before it).
  await expect(page.getByTestId("pair-viewfinder")).toHaveAttribute("data-phase", "scanning");
  await mockHook(page, "syncSimulateScanFailure", "unreachable");
  await expect(page.getByRole("status").filter({ hasText: "Couldn’t find your computer. Check that both are on the same Wi-Fi." })).toBeVisible();
  const help = page.getByTestId("hotspot-help");
  await expect(help).toContainText("Personal Hotspot");
  await expect(help).toContainText("export meetings to a file for another device");
  await expectAccessible(page);
});

test("Vietnamese copy", async ({ page }) => {
  await openSync(page, "/", { sync: "pairing", lang: "vi" });
  await page.evaluate(() => {
    window.__ghiRecord!.setOnboarding(["languages", "micPriming", "consent"] as never);
    window.location.hash = "#/onboarding";
  });
  await expect(page.getByRole("heading", { name: "Ghép nối với máy tính" })).toBeVisible();
  await expect(page.getByTestId("pair-viewfinder")).toHaveAttribute("data-phase", "scanning");
  await mockHook(page, "syncSimulateScan");
  await expect(page.getByTestId("paired-card")).toContainText("Đã ghép với MacBook Pro");
  await expect(page.getByTestId("paired-card")).toContainText("Hồ sơ giọng nói ở lại trên từng thiết bị");
  await expect(page.getByRole("button", { name: "Tiếp tục" })).toBeVisible();
});

test("without sync the step does not exist", async ({ page }) => {
  await openSync(page, "/");
  await page.evaluate(() => {
    window.__ghiRecord!.setOnboarding(["languages", "micPriming", "consent"] as never);
    window.location.hash = "#/onboarding";
  });
  await expect(step(page)).toHaveAttribute("data-step", "processing");
  await expect(page.getByText("Step 4 of 7")).toBeVisible();
  await expect(page.getByTestId("need-pair")).toHaveCount(0);
});
