// SPDX-License-Identifier: Apache-2.0
// "Ask this meeting" panel on the mocked core (copy matched as English; keys pending).
import { expect, test, type Page } from "@playwright/test";

const open = async (page: Page) => {
  await page.goto("/?platform=win#/meetings");
  await page.getByRole("button", { name: /Client call — Acme onboarding/ }).click();
  await expect(page).toHaveURL(/#\/meetings\/sample-\d+\/notes/);
  await page.getByRole("button", { name: "Ask", exact: true }).click();
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
