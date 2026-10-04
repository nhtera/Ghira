// SPDX-License-Identifier: Apache-2.0
// Settings → Custom vocabulary: add and remove round-trip through the scripted
// core, the rules match the desktop's, and the screen is accessible.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible, openApp } from "./helpers";

const calls = (page: Page, name: string) => page.evaluate((n) => window.__ghiSettingsMock!.calls[n] ?? 0, name);
const add = async (page: Page, text: string) => {
  await page.getByLabel(/^(New term|Từ mới)$/).fill(text);
  await page.getByRole("button", { name: /^(Add|Thêm)$/ }).click();
};

test("the home list opens the vocabulary and back returns", async ({ page }) => {
  await openApp(page, "/settings");
  await page.getByRole("button", { name: /^Custom vocabulary/ }).click();
  await expect(page.getByRole("heading", { name: "Custom vocabulary", level: 1 })).toBeVisible();
  await page.getByRole("button", { name: /^Back/ }).click();
  await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible();
});

test("starts empty, shows the learned names and is accessible", async ({ page }) => {
  await openApp(page, "/settings/vocabulary");
  await expect(page.getByText("No terms yet.")).toBeVisible();
  await expect(page.getByText("Linh Trần")).toBeVisible();
  await expect(page.getByTestId("vocab-count")).toHaveText("0 of 200");
  await expectAccessible(page);
});

test("a term is added, kept after leaving, and removed", async ({ page }) => {
  await openApp(page, "/settings/vocabulary");
  await add(page, "  Nguyễn   Văn An ");
  await expect(page.getByText("Nguyễn Văn An", { exact: true })).toBeVisible();
  await expect(page.getByLabel("New term")).toHaveValue("");
  await expect(page.getByTestId("vocab-count")).toHaveText("1 of 200");
  expect(await calls(page, "setVocabulary")).toBe(1);

  // Away and back: the list comes from the core.
  await page.evaluate(() => (location.hash = "#/settings"));
  await page.evaluate(() => (location.hash = "#/settings/vocabulary"));
  await expect(page.getByText("Nguyễn Văn An", { exact: true })).toBeVisible();
  await expectAccessible(page);

  await page.getByRole("button", { name: "Remove Nguyễn Văn An" }).click();
  await expect(page.getByText("No terms yet.")).toBeVisible();
  expect(await calls(page, "setVocabulary")).toBe(2);
});

test("a duplicate without accents or case is refused before the core is called", async ({ page }) => {
  await openApp(page, "/settings/vocabulary");
  await add(page, "Chốt");
  await add(page, "chot");
  await expect(page.getByText("Already in the list.")).toBeVisible();
  expect(await calls(page, "setVocabulary")).toBe(1);
});

test("a full list refuses another term", async ({ page }) => {
  await openApp(page, "/settings/vocabulary");
  await page.evaluate(() => {
    window.__ghiSettingsMock!.maxTerms = 1;
    window.__ghiSettingsMock!.terms = ["one"];
  });
  await page.evaluate(() => (location.hash = "#/settings"));
  await page.evaluate(() => (location.hash = "#/settings/vocabulary"));
  await expect(page.getByTestId("vocab-count")).toHaveText("1 of 1");
  await add(page, "two");
  await expect(page.getByText("The list is full. Remove a term to add another.")).toBeVisible();
  expect(await calls(page, "setVocabulary")).toBe(0);
});

test("a learned name is hidden while it is among the terms, even without accents", async ({ page }) => {
  await openApp(page, "/settings/vocabulary");
  await expect(page.getByRole("button", { name: "Remove Linh Trần" })).toBeVisible();
  await add(page, "Linh Tran");
  await expect(page.getByRole("button", { name: "Remove Linh Tran", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Remove Linh Trần" })).toHaveCount(0);
  await page.getByRole("button", { name: "Remove Linh Tran", exact: true }).click();
  await expect(page.getByRole("button", { name: "Remove Linh Trần" })).toBeVisible();
});

test("a removed learned name stays removed", async ({ page }) => {
  await openApp(page, "/settings/vocabulary");
  await page.getByRole("button", { name: "Remove Linh Trần" }).click();
  await add(page, "Linh Tran");
  await page.getByRole("button", { name: "Remove Linh Tran", exact: true }).click();
  await expect(page.getByText("Nothing yet. Names you give speakers appear here.")).toBeVisible();
});

test("a learned name can be removed", async ({ page }) => {
  await openApp(page, "/settings/vocabulary");
  await page.getByRole("button", { name: "Remove Linh Trần" }).click();
  await expect(page.getByText("Nothing yet. Names you give speakers appear here.")).toBeVisible();
  expect(await calls(page, "ignoreLearnedTerm")).toBe(1);
});

test("Vietnamese at 200% text stays accessible", async ({ page }) => {
  await openApp(page, "/settings/vocabulary", { lang: "vi", scale: 2 });
  await add(page, "Chốt ngân sách");
  await expect(page.getByRole("button", { name: "Xoá Chốt ngân sách" })).toBeVisible();
  await expect(page.getByTestId("vocab-count")).toHaveText("1/200");
  await expectAccessible(page);
});
