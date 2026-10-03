// SPDX-License-Identifier: Apache-2.0
// Settings (16-J): every setting round-trips through the scripted core, and
// each screen is accessible.
import { expect, test, type Page } from "@playwright/test";
import { expectAccessible, openApp } from "./helpers";

const calls = (page: Page, name: string) => page.evaluate((n) => window.__ghiSettingsMock!.calls[n] ?? 0, name);
/** Moves inside the app without reloading (the scripted core keeps its state). */
const go = (page: Page, route: string) => page.evaluate((r) => (location.hash = `#${r}`), route);
const row = (page: Page, name: string | RegExp) => page.getByRole("button", { name });

test.describe("settings home", () => {
  test("lists the areas and each opens and goes back", async ({ page }) => {
    await openApp(page, "/settings");
    await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible();
    await expectAccessible(page);
    for (const [label, title] of [
      ["Models", "Models"],
      ["Voice profile", "Voice profile"],
      ["Privacy and security", "Privacy and security"],
      ["Cloud notes", "Cloud notes"],
      ["Consent message", "Consent message"],
      ["About", "About"],
    ]) {
      await row(page, new RegExp(`^${label}`)).click();
      await expect(page.getByRole("heading", { name: title, level: 1 })).toBeVisible();
      await expectAccessible(page);
      await page.getByRole("button", { name: /^Back/ }).click();
      await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible();
    }
  });

  test("the meeting language round-trips", async ({ page }) => {
    await openApp(page, "/settings");
    await row(page, /^Vietnamese/).click();
    await expect(row(page, /^Vietnamese/).getByText("Selected")).toBeVisible();
    await expect(row(page, /^Auto/).getByText("Selected")).toHaveCount(0);
    // Away and back: the value comes from the core, not from the screen.
    await row(page, /^About/).click();
    await page.getByRole("button", { name: /^Back/ }).click();
    await expect(row(page, /^Vietnamese/).getByText("Selected")).toBeVisible();
    expect(await calls(page, "updateSettings")).toBe(1);
  });

  test("the processing target is this phone and the computer waits for pairing", async ({ page }) => {
    await openApp(page, "/settings");
    await expect(row(page, /^This phone/).getByText("Selected")).toBeVisible();
    await expect(row(page, /^My computer/)).toBeDisabled();
    await expect(page.getByText("Pair a computer first")).toBeVisible();
  });
});

test.describe("models", () => {
  test("downloads what is missing and the Wi-Fi switch round-trips", async ({ page }) => {
    await openApp(page, "/settings/models");
    await expect(page.getByText("Not downloaded")).toHaveCount(3);
    const wifi = page.getByRole("switch", { name: "Download over Wi-Fi only" });
    await expect(wifi).toBeChecked();
    await wifi.click();
    await expect(wifi).not.toBeChecked();
    await page.getByRole("button", { name: /^Download \d/ }).click();
    await expect(page.getByText("All models are ready.")).toBeVisible();
    await expect(page.getByText("Ready", { exact: true })).toHaveCount(3);
    await expectAccessible(page);
    expect(await page.evaluate(() => window.__ghiRecord!.log.filter((l) => l.startsWith("modelsDownload")))).toEqual(["modelsDownload:false"]);
  });

  test("a download waiting for Wi-Fi can use cellular this time", async ({ page }) => {
    await openApp(page, "/settings");
    await page.evaluate(() => (window.__ghiRecord!.onCellular = true));
    await go(page, "/settings/models");
    await expect(page.getByText("Waiting for Wi-Fi")).toHaveCount(3);
    await page.getByRole("button", { name: "Use cellular this time" }).click();
    await expect(page.getByText("All models are ready.")).toBeVisible();
  });
});

test.describe("voice profile", () => {
  test("enrolls only after consent, then the profile can be deleted", async ({ page }) => {
    await openApp(page, "/settings/voice");
    await expect(page.getByText("No voice profile yet.")).toBeVisible();
    const start = page.getByRole("button", { name: "Set up my voice" });
    await expect(start).toBeDisabled();
    await page.getByRole("checkbox").check();
    await start.click();
    await expect(page.getByText("Listening…")).toBeVisible();
    await expectAccessible(page);
    await page.getByRole("button", { name: "Finish" }).click();
    await expect(page.getByText("Saved on this phone · 3 samples")).toBeVisible();
    await expect(page.getByText("Voice profile saved.")).toBeVisible();
    await page.getByRole("button", { name: "Delete my voice profile" }).click();
    // Deleting a voice profile is for good: it asks first.
    const ask = page.getByRole("alertdialog");
    await ask.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByText("Saved on this phone · 3 samples")).toBeVisible();
    await page.getByRole("button", { name: "Delete my voice profile" }).click();
    await ask.getByRole("button", { name: "Delete profile" }).click();
    await expect(page.getByText("No voice profile yet.")).toBeVisible();
    await expect(page.getByText("Voice profile deleted.")).toBeVisible();
  });

  test("without the voice model it points to Models", async ({ page }) => {
    await openApp(page, "/settings");
    await page.evaluate(() => (window.__ghiRecord!.voiceModel = false));
    await go(page, "/settings/voice");
    await expect(page.getByText(/voice model isn’t downloaded/)).toBeVisible();
    await page.getByRole("button", { name: "Open Models" }).click();
    await expect(page.getByRole("heading", { name: "Models", level: 1 })).toBeVisible();
  });
});

test("leaving the voice screen mid-enrollment ends the enrollment", async ({ page }) => {
  await openApp(page, "/settings/voice");
  await page.getByRole("checkbox").check();
  await page.getByRole("button", { name: "Set up my voice" }).click();
  await expect(page.getByText("Listening…")).toBeVisible();
  await page.getByRole("button", { name: /^Back/ }).click();
  await row(page, /^Voice profile/).click();
  await expect(page.getByRole("button", { name: "Finish" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Set up my voice" })).toBeVisible();
});

test.describe("consent message", () => {
  test("saves a custom message and goes back to the built-in text", async ({ page }) => {
    await openApp(page, "/settings/consent");
    const en = page.getByLabel("English message");
    await expect(en).toHaveValue("");
    await expect(en).toHaveAttribute("placeholder", /.+/);
    const save = page.getByRole("button", { name: "Save" });
    await expect(save).toBeDisabled();
    await en.fill("This meeting is being recorded.");
    await save.click();
    await expect(page.getByText("Saved.")).toBeVisible();
    await page.getByRole("button", { name: /^Back/ }).click();
    await row(page, /^Consent message/).click();
    await expect(page.getByLabel("English message")).toHaveValue("This meeting is being recorded.");
    await page.getByRole("button", { name: "Use built-in text" }).click();
    await expect(page.getByLabel("English message")).toHaveValue("");
  });
});

test.describe("cloud notes", () => {
  test("picks a provider and keeps the API key out of sight", async ({ page }) => {
    // The long page collapses its title bar; without motion axe never sees it mid-fade.
    await page.emulateMedia({ reducedMotion: "reduce" });
    await openApp(page, "/settings/cloud");
    // Off by default: only the choice to offer cloud notes shows.
    const offer = page.getByRole("switch", { name: "Offer cloud notes" });
    await expect(offer).not.toBeChecked();
    await expect(row(page, /^Anthropic/)).toHaveCount(0);
    await offer.click();
    await expect(offer).toBeChecked();
    await expect(page.getByText("No key")).toHaveCount(0);
    await row(page, /^Anthropic/).click();
    await expect(row(page, /^Anthropic/).getByText("Selected")).toBeVisible();
    await expect(page.getByText("No key")).toBeVisible();
    const field = page.getByLabel("API key for Anthropic");
    await expect(field).toHaveAttribute("type", "password");
    await expect(field).toHaveAttribute("autocomplete", "off");
    await field.fill("sk-test-12345");
    await page.getByRole("button", { name: "Save key" }).click();
    await expect(page.getByText("Key saved to the Keychain.")).toBeVisible();
    await expect(field).toHaveValue("");
    await expect(page.getByText("Key saved", { exact: true })).toBeVisible();
    expect(await page.evaluate(() => window.__ghiSettingsMock!.lastKey)).toBe("sk-test-12345");
    // The key is never shown again, anywhere on the page.
    await expect(page.locator("body")).not.toContainText("sk-test-12345");
    await expectAccessible(page);
    await page.getByRole("button", { name: "Remove key" }).click();
    await expect(page.getByText("No key")).toBeVisible();
    // The default for redaction and the model round-trip.
    await row(page, /^gpt-5|^claude-haiku/).first().click();
    const hide = page.getByRole("switch", { name: "Hide names and personal data by default" });
    await expect(hide).toBeChecked();
    await hide.click();
    await expect(hide).not.toBeChecked();
  });

  test("no provider means cloud notes are off", async ({ page }) => {
    await openApp(page, "/settings");
    await page.evaluate(() => window.__ghiSettingsMock!.offerCloud(true));
    await go(page, "/settings/cloud");
    await expect(row(page, /^None/).getByText("Selected")).toBeVisible();
    await expect(page.getByLabel(/API key/)).toHaveCount(0);
  });
});

test.describe("about", () => {
  test("shows the version and the way to the licenses", async ({ page }) => {
    await openApp(page, "/settings/about");
    await expect(page.getByText("0.1.0").first()).toBeVisible();
    await expect(page.getByRole("button", { name: "Open-source licenses" })).toBeVisible();
    await expectAccessible(page);
  });

  test("lists what ships, filters, and opens a license text", async ({ page }) => {
    await openApp(page, "/settings/about");
    await page.getByRole("button", { name: "Open-source licenses" }).click();
    await expect(page.getByRole("heading", { level: 1, name: "Open-source software" })).toBeVisible();
    await expect(page.getByText("OpenMDW-1.1").first()).toBeVisible();
    await page.getByRole("searchbox", { name: "Search licenses" }).fill("react-dom");
    const row = page.getByRole("button", { name: /^react-dom / });
    await expect(row).toBeVisible();
    await row.click();
    await expect(page.locator("pre").first()).toContainText("Permission is hereby granted");
    await expectAccessible(page);
  });
});
