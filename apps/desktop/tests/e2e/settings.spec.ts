// SPDX-License-Identifier: Apache-2.0
// Settings on the mocked core: every section, the vocabulary, a provider key,
// the retention change and the license search.
import { expect, test, type Page } from "@playwright/test";

const open = (page: Page, section = "general") => page.goto(`/?platform=win#/settings/${section}`);
const SECTIONS = ["General", "Languages", "Recording", "AI", "Models", "Privacy", "Sync", "Shortcuts", "About"];

test("opens every section from the list", async ({ page }) => {
  await open(page);
  const nav = page.getByRole("navigation", { name: "Settings" });
  for (const name of SECTIONS) {
    await nav.getByRole("link", { name, exact: true }).click();
    await expect(page.getByRole("heading", { level: 2, name, exact: true })).toBeVisible();
  }
});

test("adds a custom vocabulary term", async ({ page }) => {
  await open(page, "languages");
  await page.getByRole("textbox", { name: "Add a term" }).fill("Ghira Beta");
  await page.getByRole("textbox", { name: "Add a term" }).press("Enter");
  await expect(page.getByRole("list", { name: "Custom vocabulary" }).getByText("Ghira Beta")).toBeVisible();
  await page.getByRole("textbox", { name: "Add a term" }).fill("ghira beta");
  await page.getByRole("textbox", { name: "Add a term" }).press("Enter");
  await expect(page.getByText("Already in the list.")).toBeVisible();
});

test("a saved key is never shown again and nothing is logged", async ({ page }) => {
  await open(page, "ai");
  await expect(page.getByTestId("log-empty")).toBeVisible();
  await page.getByLabel("Anthropic API key").fill("sk-ant-SECRET-123456");
  await page.getByRole("button", { name: "Save Anthropic key" }).click();
  const row = page.getByTestId("key-anthropic");
  await expect(row.getByText("Key saved")).toBeVisible();
  await expect(page.locator("body")).not.toContainText("sk-ant-SECRET-123456");
  expect(await page.locator("input").evaluateAll((els) => els.map((e) => (e as HTMLInputElement).value))).not.toContain("sk-ant-SECRET-123456");
  await expect(page.getByTestId("log-empty")).toBeVisible();
});

test("changing the audio retention asks first", async ({ page }) => {
  await open(page, "privacy");
  const group = page.getByRole("radiogroup", { name: "Keep audio for" });
  await group.getByRole("radio", { name: "30 days" }).click();
  const confirm = page.getByRole("alertdialog");
  await expect(confirm).toContainText("Audio older than 30 days will be deleted now");
  await expect(group.getByRole("radio", { name: "30 days" })).toHaveAttribute("aria-checked", "false");
  await confirm.getByRole("button", { name: "Delete older audio" }).click();
  await expect(group.getByRole("radio", { name: "30 days" })).toHaveAttribute("aria-checked", "true");
});

test("searches the licenses", async ({ page }) => {
  await open(page, "about");
  await page.getByRole("searchbox", { name: "Search licenses" }).fill("serde");
  const first = page.getByRole("button", { name: /^serde/ }).first();
  await expect(first).toBeVisible();
  await first.click();
  await expect(page.locator("pre").first()).not.toBeEmpty();
});

test("shows a ready update with a confirmed restart", async ({ page }) => {
  await page.goto("/?platform=win&update=1#/settings/about");
  await expect(page.getByTestId("update-line")).toContainText("is ready");
  await page.getByRole("button", { name: "Restart to update" }).click();
  await expect(page.getByRole("alertdialog")).toContainText("open again");
  await page.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByRole("button", { name: "Restart to update" })).toBeVisible();
});

test("turns the app lock on, locks now and unlocks", async ({ page }) => {
  await open(page, "privacy");
  await page.getByRole("switch", { name: "Lock Ghira with Windows Hello" }).click();
  await expect(page.getByRole("switch", { name: "Lock Ghira with Windows Hello" })).toHaveAttribute("aria-checked", "true");
  await page.getByRole("button", { name: "Lock now" }).click();
  await expect(page.getByText("Ghira is locked")).toBeVisible();
  await page.getByRole("button", { name: "Unlock with Windows Hello" }).click();
  await expect(page.getByRole("heading", { level: 2, name: "Privacy" })).toBeVisible();
});

test("the Obsidian vault folder shows by name and can be changed", async ({ page }) => {
  await open(page);
  const row = page.locator("[data-row]").filter({ hasText: "Obsidian vault folder" });
  await expect(row).toContainText("Vault");
  await row.getByRole("button", { name: /Change/ }).click();
  await expect(row).toContainText("Notes vault");
});

test("the transcript engine can be switched to Whisper and back", async ({ page }) => {
  await open(page, "models");
  const group = page.getByRole("radiogroup", { name: "Transcript after the meeting" });
  await expect(group.getByRole("radio", { name: /^Standard/ })).toHaveAttribute("aria-checked", "true");
  await group.getByRole("radio", { name: /High accuracy \(Whisper\)/ }).click();
  await expect(group.getByRole("radio", { name: /High accuracy \(Whisper\)/ })).toHaveAttribute("aria-checked", "true");
  await expect(page.getByText("Until Whisper is downloaded, transcripts use Standard.")).toBeVisible();
  await group.getByRole("radio", { name: /^Standard/ }).click();
  await expect(page.getByText("Until Whisper is downloaded, transcripts use Standard.")).toBeHidden();
});
