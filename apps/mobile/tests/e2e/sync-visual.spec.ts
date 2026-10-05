// SPDX-License-Identifier: Apache-2.0
// 15-I: the sync screens in the visual matrix (light/dark, English/Vietnamese, 200% text):
// each cell checks axe and compares against a baseline.
import { expect } from "@playwright/test";
import { visualMatrix } from "./settings-matrix";

const state = (name: string) => (p: import("@playwright/test").Page) => p.evaluate((s) => window.__ghiMock!.syncSet(s as never), name);

visualMatrix("sync", [
  {
    name: "pair-step",
    route: "/onboarding",
    setup: async (p) => {
      await state("pairing")(p);
      await p.evaluate(() => window.__ghiRecord!.setOnboarding(["languages", "micPriming", "consent"] as never));
    },
    ready: (p) => expect(p.getByTestId("pair-viewfinder")).toHaveAttribute("data-phase", "scanning"),
  },
  {
    name: "pair-step-paired",
    route: "/onboarding",
    setup: async (p) => {
      await state("paired")(p);
      await p.evaluate(() => window.__ghiRecord!.setOnboarding(["languages", "micPriming", "consent"] as never));
    },
    ready: (p) => expect(p.getByTestId("paired-card")).toBeVisible(),
  },
  { name: "settings-off", route: "/settings/sync", setup: state("off"), ready: (p) => expect(p.getByTestId("hotspot-help")).toBeVisible() },
  { name: "settings-paired", route: "/settings/sync", setup: state("paired"), ready: (p) => expect(p.getByRole("button", { name: /Sync now|Đồng bộ ngay/ })).toBeVisible() },
  { name: "settings-error", route: "/settings/sync", setup: state("error"), ready: (p) => expect(p.getByRole("alert").first()).toBeVisible() },
  { name: "settings-wipe-pending", route: "/settings/sync", setup: state("wipePending"), ready: (p) => expect(p.getByTestId("wipe-pending")).toBeVisible() },
  {
    name: "settings-unpair-sheet",
    route: "/settings/sync",
    setup: state("paired"),
    ready: async (p) => {
      await p.getByRole("button", { name: /^Unpair and delete|^Huỷ ghép và xoá/ }).click();
      await expect(p.getByRole("dialog")).toBeVisible();
    },
  },
  {
    name: "record-needs-pair",
    route: "/record",
    setup: state("off"),
    ready: (p) => expect(p.getByTestId("need-pair")).toBeVisible(),
  },
  {
    name: "meeting-leased",
    route: "/meetings/m-notes",
    setup: state("leased"),
    ready: async (p) => {
      await p.getByRole("tab", { name: /Transcript|Bản ghi/ }).click();
      await expect(p.getByTestId("audio-on-device")).toBeVisible();
      await expect(p.locator("[data-segment]").first()).toBeVisible();
    },
  },
  {
    name: "meeting-conflict",
    route: "/meetings/m-notes",
    setup: state("conflict"),
    ready: (p) => expect(p.getByTestId("conflict-banner")).toBeVisible(),
  },
  {
    name: "process-here-sheet",
    route: "/meetings/m-notes",
    setup: state("leased"),
    ready: async (p) => {
      await p.getByRole("tab", { name: /Transcript|Bản ghi/ }).click();
      await p.getByRole("button", { name: /Process on this phone now|Xử lý ngay trên điện thoại này/ }).click();
      await expect(p.getByRole("dialog")).toBeVisible();
    },
  },
]);
