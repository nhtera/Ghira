// SPDX-License-Identifier: Apache-2.0
// Speakers in the live view on the mocked core: name from the chip, merge,
// not a person, split lines, change one line's speaker.
import { expect, test, type Page } from "@playwright/test";

type Mock = { __ghiMock: { simulateCoreEvent: (e: object) => void } };

async function record(page: Page) {
  await page.goto("/?platform=win#/meetings");
  await page.getByRole("button", { name: "Record call", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "Meeting title" })).toBeVisible();
  // The scripted call introduces Me, Speaker 2 and Speaker 3.
  await expect(page.getByRole("list", { name: "Speakers" }).getByRole("button", { name: /Speaker 3/ })).toBeVisible({ timeout: 8000 });
}
const strip = (page: Page) => page.getByRole("list", { name: "Speakers" });

test("name a speaker from the chip: open, type, Enter", async ({ page }) => {
  await record(page);
  await strip(page).getByRole("button", { name: /Speaker 2/ }).click();
  await expect(page.getByRole("combobox")).toBeFocused();
  await page.keyboard.type("Hana");
  await page.keyboard.press("Enter");
  await expect(strip(page).getByRole("button", { name: /Hana/ })).toBeVisible();
  await expect(page.getByText("Renamed to Hana on every line", { exact: true })).toBeVisible();
});

test("known people are offered while typing", async ({ page }) => {
  await record(page);
  await strip(page).getByRole("button", { name: /Speaker 2/ }).click();
  await page.keyboard.type("lin");
  await page.getByRole("option", { name: "Linh" }).click();
  await expect(strip(page).getByRole("button", { name: /Linh/ })).toBeVisible();
});

test("merge into another speaker", async ({ page }) => {
  await record(page);
  await strip(page).getByRole("button", { name: /Speaker 3/ }).click();
  await page.getByRole("button", { name: "Merge into…" }).click();
  await page.getByRole("button", { name: /Speaker 2/ }).last().click();
  await expect(strip(page).getByRole("button", { name: /Speaker 3/ })).toHaveCount(0);
  await expect(strip(page).getByRole("button", { name: /Speaker 2/ })).toBeVisible();
});

test("not a person removes the speaker from the strip", async ({ page }) => {
  await record(page);
  await strip(page).getByRole("button", { name: /Speaker 3/ }).click();
  await page.getByRole("button", { name: "Not a person (video, music)" }).click();
  await expect(strip(page).getByRole("button", { name: /Speaker 3/ })).toHaveCount(0);
});

test("the voice option is not offered (third-party profiles are off)", async ({ page }) => {
  await record(page);
  await strip(page).getByRole("button", { name: /Speaker 2/ }).click();
  await expect(page.getByRole("combobox")).toBeVisible();
  await expect(page.getByRole("checkbox")).toHaveCount(0);
});

const addLines = (page: Page) =>
  page.evaluate(() => {
    const emit = (window as unknown as Mock).__ghiMock.simulateCoreEvent;
    for (const i of [1, 2]) emit({ type: "transcriptFinal", track: 0, meeting: "", line: { gid: `split-${i}`, speaker: 2, t0Ms: 200_000 + i * 3000, t1Ms: 201_500 + i * 3000, text: `split candidate ${i}`, overlap: false, words: [] } });
  });

test("split two lines off to a new speaker", async ({ page }) => {
  await record(page);
  await addLines(page);
  await strip(page).getByRole("button", { name: /Speaker 2/ }).click();
  await page.getByRole("button", { name: "Split speaker…" }).click();
  await page.getByRole("checkbox", { name: "split candidate 1" }).check();
  await page.getByRole("checkbox", { name: "split candidate 2" }).check();
  await page.getByRole("button", { name: "Move 2 lines" }).click();
  await expect(page.getByText(/^Moved 2 lines to/)).toBeVisible();
  await expect(strip(page).getByRole("button")).toHaveCount(3);
});

test("change one line's speaker from the line", async ({ page }) => {
  await record(page);
  await addLines(page);
  const row = page.getByRole("main").locator("ol > li").filter({ hasText: "split candidate 2" });
  await row.hover();
  await row.getByRole("button", { name: "Change speaker" }).click();
  await page.getByRole("button", { name: /Speaker 3/ }).last().click();
  await expect(page.getByText(/^Merged into/)).toBeVisible();
  // Focus returns to the line's button, not to the page.
  await expect(row.getByRole("button", { name: "Change speaker" })).toBeFocused();
});
