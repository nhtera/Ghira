// SPDX-License-Identifier: Apache-2.0
// The cloud send sheet in the visual matrix (16-J).
import { expect, type Page } from "@playwright/test";
import { visualMatrix } from "./settings-matrix";

const open = (p: Page, meetingId: string) =>
  p.evaluate((id) => {
    window.__ghiSettingsMock!.keys.anthropic = true;
    window.dispatchEvent(new CustomEvent("ghi:open-cloud-sheet", { detail: { meetingId: id } }));
  }, meetingId);

visualMatrix("cloud-sheet", [
  {
    name: "preview",
    route: "/settings",
    ready: async (p) => {
      await open(p, "m-1");
      await expect(p.locator("pre")).toBeVisible();
    },
  },
  {
    name: "redaction-off",
    route: "/settings",
    ready: async (p) => {
      await open(p, "m-1");
      await expect(p.locator("pre")).toBeVisible();
      // A DOM click: Playwright's own click scrolls the switch into view by a timing-dependent amount at 200% text.
      await p.getByRole("dialog").getByRole("switch").evaluate((el) => (el as HTMLElement).click());
      await expect(p.locator("pre")).toContainText("Nguyễn Văn An");
    },
  },
  {
    name: "locked",
    route: "/settings",
    setup: async (p) => {
      await p.evaluate(() => window.__ghiSettingsMock!.cloudLocked.push("locked-1"));
    },
    ready: async (p) => {
      await open(p, "locked-1");
      await expect(p.getByRole("dialog").getByRole("button", { name: /^(Send|Gửi)$/ })).toBeDisabled();
      await expect(p.getByRole("dialog").locator("[data-cloud-state=locked]")).toBeVisible();
    },
  },
]);
