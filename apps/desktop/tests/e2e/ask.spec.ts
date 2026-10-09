// SPDX-License-Identifier: Apache-2.0
// "Ask this meeting" panel on the mocked core (copy matched as English; keys pending).
import { expect, test, type Page } from "@playwright/test";

const open = async (page: Page) => {
  await page.goto("/?platform=win#/meetings");
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await expect(page).toHaveURL(/#\/meetings\/sample-\d+\/notes/);
  // Ask lives in the Export menu of the meeting toolbar.
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("menuitem", { name: "Ask", exact: true }).click();
  return page.getByRole("complementary", { name: "Ask this meeting" });
};

test("an answered question has a citation chip that plays the moment", async ({ page }) => {
  const panel = await open(page);
  await panel.getByRole("textbox", { name: "Question" }).fill("nhận diện");
  await page.keyboard.press("Enter");
  const entry = panel.getByTestId("ask-entry");
  await expect(entry.getByText("Answered on this device")).toBeVisible();
  const before = Number(await page.getByTestId("meeting-detail").getAttribute("data-player-ms"));
  await entry.getByRole("button", { name: /^Show in transcript \d+:\d\d$/ }).click();
  await expect.poll(async () => Number(await page.getByTestId("meeting-detail").getAttribute("data-player-ms"))).not.toBe(before);
});

test("a question nothing matches is a Not discussed card with the searched terms", async ({ page }) => {
  const panel = await open(page);
  await panel.getByRole("textbox", { name: "Question" }).fill("kubernetes");
  await page.keyboard.press("Enter");
  const card = panel.getByTestId("ask-not-discussed");
  await expect(card.getByText("Not discussed in this meeting")).toBeVisible();
  await expect(card).toContainText("kubernetes");
});

test("the empty state offers three starter questions that send", async ({ page }) => {
  const panel = await open(page);
  const starters = panel.getByRole("list", { name: "Try asking" }).getByRole("button");
  await expect(starters).toHaveCount(3);
  await starters.first().click();
  const entry = panel.getByTestId("ask-entry");
  await expect(entry.getByText("What was decided?")).toBeVisible();
  await expect(panel.getByRole("list", { name: "Try asking" })).toHaveCount(0);
});

test("a starter sends with Enter from the keyboard", async ({ page }) => {
  const panel = await open(page);
  await panel.getByRole("button", { name: "What were the main themes?" }).focus();
  await page.keyboard.press("Enter");
  await expect(panel.getByTestId("ask-entry").getByText("What were the main themes?")).toBeVisible();
});

