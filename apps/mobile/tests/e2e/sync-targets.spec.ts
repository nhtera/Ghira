// SPDX-License-Identifier: Apache-2.0
// 15-I: the Desktop processing target on the scripted mock: pickable once a
// computer is paired (record screen, import inbox), off with "Pair a computer first" otherwise.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible } from "./helpers";
import { startRecording } from "./record-support";
import { openSync } from "./sync-support";

const item = (id: string) => ({ id, name: `${id}.m4a`, sizeBytes: 12_000_000, language: "auto", target: "phone", state: "pending", reason: null });
const seed = (page: Page, items: unknown[]) => page.evaluate((i) => window.__ghiSettingsMock!.setInbox(i as never), items);

test.describe("record screen", () => {
  test("paired: My computer can be picked and the recording goes there", async ({ page }) => {
    await openSync(page, "/record", { sync: "paired" });
    const mine = page.getByRole("radio", { name: "My computer" });
    await expect(mine).toBeEnabled();
    await expect(page.getByRole("radio", { name: "Cloud · off" })).toBeDisabled();
    await expect(page.getByTestId("need-pair")).toHaveCount(0);
    await mine.check({ force: true });
    await expect(mine).toBeChecked();
    await expectAccessible(page);
    await startRecording(page);
    await page.getByRole("button", { name: /^Pause/ }).first().waitFor();
    expect(await page.evaluate(() => window.__ghiRecord!.lastStart?.target)).toBe("desktop");
  });

  test("not paired: it is off, says to pair first, and links to the pairing screen", async ({ page }) => {
    await openSync(page, "/record", { sync: "off" });
    await expect(page.getByRole("radio", { name: "My computer" })).toBeDisabled();
    await expect(page.getByTestId("need-pair")).toContainText("Pair a computer first");
    await expectAccessible(page);
    const link = page.getByTestId("need-pair").getByRole("button", { name: "Pair a computer" });
    expect((await link.boundingBox())!.height).toBeGreaterThanOrEqual(44);
    await link.click();
    await expect(page.getByRole("heading", { level: 1, name: "Sync with computer" })).toBeVisible();
  });

  test("without sync nothing changes: off, and no note", async ({ page }) => {
    await openSync(page, "/record");
    await expect(page.getByRole("radio", { name: "My computer" })).toBeDisabled();
    await expect(page.getByTestId("need-pair")).toHaveCount(0);
  });
});

test.describe("import inbox", () => {
  test("paired: My computer is a choice", async ({ page }) => {
    await openSync(page, "/settings", { sync: "paired" });
    await seed(page, [item("standup")]);
    await page.getByRole("status").filter({ hasText: "waiting to import" }).getByRole("button", { name: "Review" }).click();
    const dialog = page.getByRole("dialog", { name: "Waiting to import" });
    const mine = dialog.getByRole("button", { name: "My computer" });
    await expect(mine).toBeEnabled();
    await mine.click();
    await expect(mine).toHaveAttribute("aria-pressed", "true");
    await dialog.getByRole("button", { name: "Import", exact: true }).click();
    await expect(dialog.getByText("Importing…")).toBeVisible();
  });

  test("not paired: off with the reason", async ({ page }) => {
    await openSync(page, "/settings", { sync: "off" });
    await seed(page, [item("standup")]);
    await page.getByRole("status").filter({ hasText: "waiting to import" }).getByRole("button", { name: "Review" }).click();
    const dialog = page.getByRole("dialog", { name: "Waiting to import" });
    await expect(dialog.getByRole("button", { name: "My computer" })).toBeDisabled();
    await expect(dialog.getByTestId("need-pair")).toHaveText("Pair a computer first");
    await expectAccessible(page);
  });
});
