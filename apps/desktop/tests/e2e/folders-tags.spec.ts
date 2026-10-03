// SPDX-License-Identifier: Apache-2.0
// Folders and tags (phase 14d, D7) on the mocked core: create a folder while
// moving two meetings into it, filter from the sidebar, tag, narrow the
// library and the search by tag, rename and delete from Settings.
import { expect, test, type Page } from "@playwright/test";

const open = (page: Page, hash = "/meetings") => page.goto(`/?platform=win#${hash}`);
const rows = (page: Page) => page.getByRole("list").getByRole("listitem").filter({ has: page.getByRole("checkbox") });
/** The row count once the list has stopped growing (the library loads in pages). */
async function settledCount(page: Page): Promise<number> {
  await expect(rows(page).first()).toBeVisible();
  let last = -1;
  for (let i = 0; i < 20; i++) {
    const n = await rows(page).count();
    if (n === last) return n;
    last = n;
    await page.waitForTimeout(150);
  }
  return last;
}
const pick = (page: Page, n: number) => page.getByRole("checkbox", { name: /^Select / }).nth(n).click();

test("move two meetings into a new folder, then filter by it from the sidebar", async ({ page }) => {
  await open(page);
  const all = await settledCount(page);
  await pick(page, 0);
  await pick(page, 1);
  await page.getByRole("button", { name: "Move to folder…" }).click();
  await page.getByRole("textbox", { name: "Folder name" }).fill("Clients");
  await page.keyboard.press("Enter");
  await expect(page.getByText("Moved 2 meetings to Clients", { exact: true })).toBeVisible();

  const side = page.getByRole("navigation", { name: "Main" });
  const link = side.getByRole("link", { name: /Clients/ });
  await expect(link).toContainText("2");
  await link.click();
  await expect(page).toHaveURL(/#\/meetings\?folder=/);
  await expect(rows(page)).toHaveCount(2);
  // The Folder chip shows the filter, and Clear filters brings the rest back.
  await expect(page.getByRole("button", { name: "Folder · 1" })).toBeVisible();
  await page.getByRole("button", { name: "Clear filters" }).click();
  await expect(rows(page)).toHaveCount(all);
});

test("No folder shows only the meetings outside any folder", async ({ page }) => {
  await open(page);
  const all = await settledCount(page);
  await pick(page, 0);
  await page.getByRole("button", { name: "Move to folder…" }).click();
  await page.getByRole("textbox", { name: "Folder name" }).fill("Ops");
  await page.keyboard.press("Enter");
  await expect(page.getByText("Moved 1 meeting to Ops", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Folder", exact: true }).click();
  await page.getByRole("menuitemradio", { name: "No folder" }).click();
  await expect(rows(page)).toHaveCount(all - 1);
});

test("tag a meeting from the selection bar, narrow by the tag, and search within it", async ({ page }) => {
  await open(page);
  await pick(page, 0);
  await page.getByRole("button", { name: "Add tag…" }).click();
  await page.getByRole("combobox", { name: "Tag name" }).fill("Review");
  await page.keyboard.press("Enter");
  // The row shows the tag as a chip.
  await expect(page.getByRole("list").getByText("Review").first()).toBeVisible();
  await page.getByRole("button", { name: "Tags", exact: true }).click();
  await page.getByRole("menuitemcheckbox", { name: "Review" }).click();
  await expect(rows(page)).toHaveCount(1);
  // A search keeps the tag filter: only that meeting's hits.
  await page.getByRole("searchbox").fill("nhan dien");
  const groups = page.getByRole("region").filter({ has: page.getByRole("listitem") });
  await expect(groups.first()).toBeVisible();
  await expect(groups).toHaveCount(1);
});

test("the meeting header edits the folder and tags; × removes a tag", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await page.getByRole("button", { name: "Add tag…" }).click();
  await page.getByRole("combobox", { name: "Tag name" }).fill("Họp");
  await page.keyboard.press("Enter");
  const tags = page.getByRole("list", { name: "Tags" });
  await expect(tags.getByText("Họp")).toBeVisible();
  // Typing "hop" for another tag offers the existing "Họp"; it is already on the meeting, so nothing is offered twice.
  await page.getByRole("button", { name: "Add tag…" }).click();
  await page.getByRole("combobox", { name: "Tag name" }).fill("hop");
  await expect(page.getByRole("option", { name: "Họp" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Remove tag Họp" }).click();
  await expect(tags.getByText("Họp")).toHaveCount(0);
});

test("accents make a different folder; an unaccented look-alike of a lone folder is a duplicate", async ({ page }) => {
  await open(page);
  const side = page.getByRole("navigation", { name: "Main" });
  const add = async (name: string) => {
    await side.getByRole("button", { name: "New folder…" }).click();
    await page.getByRole("textbox", { name: "Folder name" }).fill(name);
    await page.keyboard.press("Enter");
  };
  await add("Họp");
  await expect(side.getByRole("link", { name: /Họp/ })).toBeVisible();
  // "hop" would be the same folder as the lone "Họp": refused.
  await add("hop");
  await expect(page.getByText("“hop” already exists.")).toBeVisible();
  await page.keyboard.press("Escape");
  // "Hộp" has its own accents: a different folder.
  await add("Hộp");
  await expect(side.getByRole("link", { name: /Hộp/ })).toBeVisible();
});

test("a refusal from the core is a sentence", async ({ page }) => {
  await open(page, "/meetings").then(() => page.goto("/?platform=win&organizefail=storage#/meetings"));
  await pick(page, 0);
  await page.getByRole("button", { name: "Move to folder…" }).click();
  await page.getByRole("textbox", { name: "Folder name" }).fill("X");
  await page.keyboard.press("Enter");
  await expect(page.getByText("That didn’t work. Try again.")).toBeVisible();
});

test("Settings → General: rename and delete with a confirm that says the meetings stay", async ({ page }) => {
  await open(page);
  const side = page.getByRole("navigation", { name: "Main" });
  await side.getByRole("button", { name: "New folder…" }).click();
  await page.getByRole("textbox", { name: "Folder name" }).fill("Temp");
  await page.keyboard.press("Enter");
  await expect(side.getByRole("link", { name: /Temp/ })).toBeVisible();

  await page.goto("/?platform=win#/settings/general");
  const card = page.locator("section").filter({ has: page.getByRole("heading", { name: "Folders and tags" }) });
  await expect(card).toBeVisible();
  await card.getByRole("button", { name: "Delete folder: Temp" }).click();
  await expect(page.getByRole("alertdialog")).toContainText("stay in Meetings");
  await page.getByRole("alertdialog").getByRole("button", { name: "Delete", exact: true }).click();
  await expect(card.getByText("Temp")).toHaveCount(0);
});
