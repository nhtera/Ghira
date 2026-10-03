// SPDX-License-Identifier: Apache-2.0
// Settings screens in the visual matrix (16-J).
import { expect } from "@playwright/test";
import { visualMatrix } from "./settings-matrix";

visualMatrix("settings", [
  { name: "home", route: "/settings", ready: (p) => expect(p.getByRole("heading", { level: 1 })).toBeVisible() },
  { name: "models", route: "/settings/models", ready: (p) => expect(p.getByRole("progressbar")).toHaveCount(0) },
  {
    name: "voice-enroll",
    route: "/settings/voice",
    ready: (p) => expect(p.getByRole("checkbox")).toBeVisible(),
  },
  {
    name: "cloud",
    route: "/settings/cloud",
    setup: async (p) => {
      await p.evaluate(() => (window.__ghiSettingsMock!.keys.anthropic = true));
    },
    ready: async (p) => {
      // Pick the provider so the model, redaction and key sections show.
      await p.getByRole("button", { name: /^Anthropic/ }).click();
      await expect(p.locator("#cloud-key")).toBeVisible();
    },
  },
  {
    name: "consent",
    route: "/settings/consent",
    ready: (p) => expect(p.locator("textarea").first()).toBeVisible(),
  },
  { name: "about", route: "/settings/about", ready: (p) => expect(p.getByText("0.1.0").first()).toBeVisible() },
]);
