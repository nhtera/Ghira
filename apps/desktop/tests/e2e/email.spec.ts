// SPDX-License-Identifier: Apache-2.0
// Follow-up email draft from the meeting detail's Share menu, on the mocked core.
import { expect, test } from "@playwright/test";

test("detail → Share → Follow-up email → Write → Copy", async ({ page, context, browserName }) => {
  // WebKit has no clipboard permission to grant: there the toast is checked.
  const readable = browserName === "chromium";
  if (readable) await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto("/?platform=win#/meetings");
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("menuitem", { name: "Draft follow-up email" }).click();
  const dialog = page.getByRole("dialog", { name: "Draft follow-up email" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("radio", { name: "Formal" }).click();
  await dialog.getByRole("button", { name: "Write draft" }).click();
  await expect(dialog.getByRole("status")).toContainText("Writing");
  const subject = dialog.getByRole("textbox", { name: "Subject" });
  await expect(subject).not.toHaveValue("");
  await expect(dialog.getByText(/Nothing is sent from Ghira/)).toBeVisible();
  await dialog.getByRole("button", { name: "Copy email" }).click();
  await expect(page.getByText("Email copied", { exact: true })).toBeVisible();
  if (readable) {
    const clip = await page.evaluate(() => navigator.clipboard.readText());
    expect(clip).toContain("\n\n");
  }
});
