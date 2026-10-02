// SPDX-License-Identifier: Apache-2.0
// Export sheet from the library bulk bar and a row's quick action, on the mocked core.
import { expect, test } from "@playwright/test";

test("bulk export to Markdown ends in a Saved toast", async ({ page }) => {
  await page.goto("/?platform=win#/meetings");
  const boxes = page.getByRole("checkbox", { name: /^Select / });
  await boxes.nth(0).click();
  await boxes.nth(1).click();
  await page.getByRole("toolbar").getByRole("button", { name: "Export…" }).click();
  const sheet = page.getByRole("dialog", { name: "Export notes" });
  await expect(sheet).toBeVisible();
  await expect(sheet.getByRole("radio", { name: "Markdown (.md)" })).toHaveAttribute("aria-checked", "true");
  await sheet.getByRole("button", { name: "Export…" }).click();
  await expect(page.getByText("Saved 2 meetings", { exact: true })).toBeVisible();
  await expect(sheet).toHaveCount(0);
});

test("a single meeting: copy as Markdown, subtitles lock the parts", async ({ page }) => {
  await page.goto("/?platform=win#/meetings");
  const row = page.getByRole("listitem").filter({ hasText: "Client call — Acme onboarding" });
  await row.getByRole("button", { name: "Export", exact: true }).click({ force: true });
  const sheet = page.getByRole("dialog", { name: "Export notes" });
  await expect(sheet).toBeVisible();
  await sheet.getByRole("radio", { name: "Subtitles (.srt)" }).click();
  await expect(sheet.getByRole("checkbox", { name: "Notes" })).toBeDisabled();
  await expect(sheet.getByText("Subtitles contain speaker names and transcript only.")).toBeVisible();
  await sheet.getByRole("radio", { name: "Word document (.docx)" }).click();
  await sheet.getByRole("button", { name: "Export…" }).click();
  await expect(page.getByText(/^Saved .*\.docx$/)).toBeVisible();
});
