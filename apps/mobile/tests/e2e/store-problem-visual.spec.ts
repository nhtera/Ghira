// SPDX-License-Identifier: Apache-2.0
// The can't-open and couldn't-load screens in the visual matrix: light and
// dark, English and Vietnamese, 200% text.
import { expect } from "@playwright/test";
import { visualMatrix } from "./settings-matrix";

const show = (problem: string) => (p: import("@playwright/test").Page) =>
  p.evaluate((pr) => {
    window.__ghiSettingsMock!.storeProblem = pr as never;
    window.dispatchEvent(new Event("focus"));
  }, problem);

visualMatrix("store-problem", [
  {
    name: "key-missing",
    route: "/settings",
    setup: show("keyMissing"),
    ready: async (p) => {
      await expect(p.getByTestId("store-problem-code")).toBeVisible();
    },
  },
  {
    name: "confirm",
    route: "/settings",
    setup: show("damaged"),
    ready: async (p) => {
      await p.getByTestId("store-problem-gate").locator("button").last().click();
      await expect(p.locator("#start-fresh-confirm")).toBeVisible();
    },
  },
]);
