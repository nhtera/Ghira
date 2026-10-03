// SPDX-License-Identifier: Apache-2.0
// The share-inbox banner and sheet in the visual matrix (16-J).
import { expect, type Page } from "@playwright/test";
import { visualMatrix } from "./settings-matrix";

const items = [
  { id: "standup", name: "weekly-standup.m4a", sizeBytes: 48_000_000, language: "auto", target: "phone", state: "pending", reason: null },
  { id: "memo", name: "voice-memo.caf", sizeBytes: 3_500_000, language: "auto", target: "phone", state: "rejected", reason: "unsupportedType" },
  { id: "call", name: "họp-quý-4.mp3", sizeBytes: null, language: "vi", target: "phone", state: "importing", reason: null },
];
const seed = (p: Page) => p.evaluate((i) => window.__ghiSettingsMock!.setInbox(i as never), items);

visualMatrix("inbox", [
  {
    name: "banner",
    route: "/settings",
    ready: async (p) => {
      await seed(p);
      await expect(p.getByRole("button", { name: /^(Review|Xem)$/ })).toBeVisible();
    },
  },
  {
    name: "sheet",
    route: "/settings",
    ready: async (p) => {
      await seed(p);
      await p.getByRole("button", { name: /^(Review|Xem)$/ }).click();
      await expect(p.getByRole("dialog").getByText("weekly-standup.m4a")).toBeVisible();
    },
  },
]);
