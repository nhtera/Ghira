// SPDX-License-Identifier: Apache-2.0
// Search: accent-insensitive, highlight on the original text, recent queries.
import { expect, test } from "@playwright/test";
import { expectAccessible } from "./helpers";
import { openMeetings } from "./meetings-helpers";

test.describe("search", () => {
  test("explains itself before a query", async ({ page }) => {
    await openMeetings(page, "/search");
    await expect(page.getByText(/“dong” finds “đồng”/)).toBeVisible();
  });

  test("dong finds đồng and marks it with its accents", async ({ page }) => {
    await openMeetings(page, "/search");
    await page.getByRole("searchbox").fill("dong");
    await expect(page.getByText(/\d results?/)).toBeVisible();
    const marks = page.locator("mark");
    await expect(marks.first()).toHaveText("đồng");
    for (const m of await marks.all())
      expect(await m.textContent()).toBe("đồng");
    await expect(page.getByText("Họp kế hoạch quý 4 · Đà Nẵng")).toBeVisible();
    await expect(page.getByText("Product sync tuần 39")).toBeVisible();
  });

  test("da nang finds Đà Nẵng and da nang", async ({ page }) => {
    await openMeetings(page, "/search");
    await page.getByRole("searchbox").fill("da nang");
    await expect(page.locator("mark").first()).toBeVisible();
    const texts = await page.locator("mark").allTextContents();
    expect(texts).toContain("Đà Nẵng");
    expect(texts).toContain("da nang");
  });

  test("an unaccented query matches the accented word, VI too", async ({
    page,
  }) => {
    await openMeetings(page, "/search", { lang: "vi" });
    await page.getByRole("searchbox").fill("DONG");
    await expect(page.locator("mark").first()).toHaveText("đồng");
    await expect(page.getByText(/\d kết quả/)).toBeVisible();
  });

  test("no matches", async ({ page }) => {
    await openMeetings(page, "/search");
    await page.getByRole("searchbox").fill("zzzz");
    await expect(
      page.getByRole("heading", { name: "No matches for “zzzz”" }),
    ).toBeVisible();
  });

  test("a result opens the transcript at that line; the query becomes a recent one", async ({
    page,
  }) => {
    await openMeetings(page, "/search");
    await page.getByRole("searchbox").fill("da nang");
    await page
      .getByRole("button", { name: /Họp kế hoạch quý 4/ })
      .first()
      .click();
    await expect(page).toHaveURL(/#\/meetings\/m-nonotes\?.*tab=transcript/);
    await expect(
      page
        .locator(
          "[data-segment][data-selected], [data-segment] [data-selected]",
        )
        .first(),
    ).toBeVisible();
    // Back to Search (the tab), the query is a recent search; stored only in the page.
    await page.getByRole("button", { name: "Search" }).click();
    await expect(
      page.getByRole("heading", { name: "Recent searches" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "da nang" }).click();
    await expect(page.getByRole("searchbox")).toHaveValue("da nang");
    expect(
      await page.evaluate(() => localStorage.getItem("ghi.search.recent")),
    ).toBe('["da nang"]');
    await page.getByRole("button", { name: "Clear search" }).click();
    await page.getByRole("button", { name: "Clear", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "Recent searches" }),
    ).toHaveCount(0);
    expect(
      await page.evaluate(() => localStorage.getItem("ghi.search.recent")),
    ).toBeNull();
  });

  test("is accessible in both languages", async ({ page }) => {
    await openMeetings(page, "/search");
    await page.getByRole("searchbox").fill("dong");
    await expect(page.locator("mark").first()).toBeVisible();
    await expectAccessible(page);
    await openMeetings(page, "/search", { lang: "vi", scale: 2 });
    await page.getByRole("searchbox").fill("dong");
    await expect(page.locator("mark").first()).toBeVisible();
    await expectAccessible(page);
  });
});
