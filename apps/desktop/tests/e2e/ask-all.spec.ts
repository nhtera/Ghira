// SPDX-License-Identifier: Apache-2.0
// D9: Ask across meetings, and "Related by meaning" in library search, on the mocked core.
import { expect, test } from "@playwright/test";

test("an answer across meetings has chips; a chip opens that meeting at the moment", async ({ page }) => {
  await page.goto("/?platform=win#/ask");
  await expect(page.getByTestId("ask-scope-line")).toContainText(/Searching \d+ meetings/);
  await page.getByRole("textbox", { name: "Question" }).fill("nhận diện");
  await page.keyboard.press("Enter");
  const entry = page.getByTestId("ask-entry");
  await expect(entry.getByText(/Answered on this PC · .+ · \d+ meetings? read/)).toBeVisible();
  const chips = entry.getByTestId("ask-chip");
  await expect(chips.first()).toBeVisible();
  expect(await chips.count()).toBeGreaterThanOrEqual(2);
  // The chip names a meeting and a moment: it opens that meeting at that time.
  const label = (await chips.first().getAttribute("aria-label")) ?? "";
  const m = /^Open (.+) at (\d+):(\d\d)$/.exec(label);
  expect(m).not.toBeNull();
  const tMs = (Number(m![2]) * 60 + Number(m![3])) * 1000;
  await chips.first().click();
  await expect(page).toHaveURL(new RegExp(`#/meetings/[^/]+/transcript\\?t=${tMs}$`));
  await expect(page.getByRole("textbox", { name: "Meeting title" })).toHaveValue(m![1]!);
});

test("pricing is not discussed and the button searches the library", async ({ page }) => {
  await page.goto("/?platform=win#/ask");
  await page.getByRole("textbox", { name: "Question" }).fill("pricing tiers");
  await page.keyboard.press("Enter");
  const card = page.getByTestId("ask-not-discussed");
  await expect(card.getByText("Not discussed in these meetings")).toBeVisible();
  await card.getByRole("button", { name: /Search transcripts for/ }).click();
  await expect(page).toHaveURL(/#\/meetings/);
  await expect(page.getByRole("searchbox")).toHaveValue("pricing");
});

for (const [code, text] of [
  ["busyRecording", /waits until the recording stops/],
  ["busyNotes", /Notes are being written/],
  ["noModel", /notes model isn.t installed/],
] as const) {
  test(`a refusal (${code}) is a neutral note, not an alert`, async ({ page }) => {
    await page.goto(`/?platform=win&askfail=${code}#/ask`);
    await page.getByRole("textbox", { name: "Question" }).fill("nhận diện");
    await page.keyboard.press("Enter");
    await expect(page.getByTestId("ask-busy")).toContainText(text);
    await expect(page.getByRole("alert")).toHaveCount(0);
  });
}

test("keyword-only note when meaning search is off", async ({ page }) => {
  await page.goto("/?platform=win&keywordonly=1#/ask");
  await page.getByRole("textbox", { name: "Question" }).fill("nhận diện");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("ask-keyword-only")).toBeVisible();
});

test("library search shows Related by meaning for words no transcript contains", async ({ page }) => {
  await page.goto("/?platform=win#/meetings");
  await page.getByRole("searchbox").fill("quarterly budget");
  const related = page.getByTestId("related-section");
  await expect(related).toBeVisible();
  await expect(related.getByText("Related by meaning")).toBeVisible();
  await related.getByRole("button").first().click();
  await expect(page).toHaveURL(/#\/meetings\/[^/]+\/(transcript|notes)/);
});

test("related meetings never repeat a meeting that already has keyword hits", async ({ page }) => {
  await page.goto("/?platform=win#/meetings");
  await page.getByRole("searchbox").fill("nhan dien");
  await expect(page.getByText(/\d+ results?/)).toBeVisible();
  await page.waitForTimeout(900); // the related list waits 600 ms
  const hitTitles = await page.getByRole("region").evaluateAll((els) => els.filter((e) => !e.matches("[data-testid=related-section]")).map((e) => e.getAttribute("aria-label")));
  const related = page.getByTestId("related-section").getByRole("button");
  const relatedTitles = await related.evaluateAll((els) => els.map((e) => e.querySelector("span span")?.textContent));
  for (const t of relatedTitles) expect(hitTitles).not.toContain(t);
});

test("no related list when meaning search is off", async ({ page }) => {
  await page.goto("/?platform=win&keywordonly=1#/meetings");
  await page.getByRole("searchbox").fill("quarterly budget");
  await page.waitForTimeout(900);
  await expect(page.getByTestId("related-section")).toHaveCount(0);
});

test("A person scope: pick someone, the scope line and the answer's footer name them", async ({ page }) => {
  await page.goto("/?platform=win#/ask");
  await page.getByRole("radio", { name: "A person" }).click();
  await page.getByRole("button", { name: "Choose a person" }).click();
  await page.getByRole("menuitem", { name: "Linh" }).click();
  await expect(page.getByTestId("ask-scope-line")).toContainText(/Searching \d+ meetings? with Linh/);
  await page.getByRole("textbox", { name: "Question" }).fill("nhận diện");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("ask-entry").getByText(/meetings? read · Linh/)).toBeVisible();
});
