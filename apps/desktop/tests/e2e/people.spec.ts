// SPDX-License-Identifier: Apache-2.0
// D8 People on the mocked core: the list, the two separate confirms, merge,
// refusals, and the empty state.
import { expect, test, type Page } from "@playwright/test";

const open = (page: Page, query = "") => page.goto(`/?platform=win${query}#/people`);
const list = (page: Page) => page.getByRole("navigation", { name: "People" });

test("Me comes first; a person's page has samples, open items and meetings", async ({ page }) => {
  await open(page);
  const rows = list(page).getByRole("button");
  await expect(rows.first()).toContainText("Me");
  await expect(rows.nth(1)).toContainText("Linh");
  await rows.nth(1).click();
  await expect(page.getByRole("heading", { name: "Linh", level: 2 })).toBeVisible();
  await expect(page.getByRole("region", { name: "Voice samples" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Open action items" })).toBeVisible();
  await page.getByRole("region", { name: "Meetings together" }).getByRole("button").first().click();
  await expect(page).toHaveURL(/#\/meetings\/[^/]+\/notes/);
});

test("Remove name from notes: the count is shown and the person goes (no voice profile to keep)", async ({ page }) => {
  await open(page);
  await list(page).getByRole("button", { name: /Linh/ }).click();
  await expect(page.getByRole("heading", { name: "Linh", level: 2 })).toBeVisible();
  await page.getByRole("button", { name: "Remove name from notes…" }).click();
  await expect(page.getByRole("alertdialog")).toContainText(/Replace “Linh” with “Speaker N” in \d+ meetings?/);
  // Only the name confirm is open: voice data is a separate control with its own confirm.
  await expect(page.getByRole("alertdialog")).toHaveCount(1);
  await page.getByRole("button", { name: "Remove name", exact: true }).click();
  await expect(page.getByText(/Name removed in \d+ meetings?\./)).toBeVisible();
  await expect(list(page).getByRole("button", { name: /Linh/ })).toHaveCount(0);
  // The first row (Me) is shown, not an error for the person who just went away.
  await expect(page.getByRole("heading", { name: "Me", level: 2 })).toBeVisible();
});

test("Delete voice data (Me) is a separate confirm and leaves everything else", async ({ page }) => {
  await open(page);
  await expect(list(page).getByRole("button").first()).toContainText("Your voice · enrolled");
  await page.getByRole("button", { name: "Delete voice data…" }).click();
  await expect(page.getByRole("alertdialog")).toHaveCount(1);
  await expect(page.getByRole("button", { name: "Remove name from notes…" })).toHaveCount(0);
  await page.getByRole("button", { name: "Delete voice data", exact: true }).click();
  await expect(page.getByText(/^Voice data deleted/)).toBeVisible();
  await expect(list(page).getByRole("button").first()).toContainText("No voice profile");
  await expect(page.getByRole("button", { name: "Delete voice data…" })).toHaveCount(0);
});

test("Escape closes a confirm and nothing is deleted", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Delete voice data…" }).click();
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Delete voice data…" })).toBeVisible();
});

test("merging asks first, then merges into the target", async ({ page }) => {
  await open(page);
  await list(page).getByRole("button", { name: /Minh/ }).click();
  await page.getByRole("button", { name: "Merge with…" }).click();
  await page.getByRole("menuitem", { name: "Linh" }).click();
  await expect(page.getByRole("alertdialog")).toContainText("Merge Minh into Linh?");
  await page.getByRole("alertdialog").getByRole("button", { name: "Merge", exact: true }).click();
  await expect(page.getByText("Merged Minh into Linh")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Linh", level: 2 })).toBeVisible();
  await expect(list(page).getByRole("button", { name: /Minh/ })).toHaveCount(0);
});

test("a refusal is a sentence, not a code", async ({ page }) => {
  await open(page, "&peopleerr=busyRecording");
  await list(page).getByRole("button", { name: /Minh/ }).click();
  await page.getByRole("button", { name: "Merge with…" }).click();
  await page.getByRole("menuitem", { name: "Linh" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Merge", exact: true }).click();
  await expect(page.getByText("This waits until the recording stops.")).toBeVisible();
});

test("no meetings yet: the empty state", async ({ page }) => {
  await open(page, "&peopleempty=1");
  await expect(page.getByText("People appear after your first meeting")).toBeVisible();
});

test("Me can record their voice again from People", async ({ page }) => {
  await open(page, "&enrollspeed=20");
  await page.getByRole("button", { name: "Record your voice again…" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("checkbox").check();
  await dialog.getByRole("button", { name: "Start reading" }).click();
  // Stop only once the (sped-up) reading has enough speech: the core refuses under 10 s.
  await expect(dialog.getByText(/Reading… (1\d|2\d) s/)).toBeVisible();
  await dialog.getByRole("button", { name: "Stop and save" }).click();
  await expect(dialog.getByText("Your voice is saved")).toBeVisible();
});
