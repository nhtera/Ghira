// SPDX-License-Identifier: Apache-2.0
// Privacy screen and its sheets in the visual matrix (16-J).
import { expect } from "@playwright/test";
import { visualMatrix } from "./settings-matrix";

visualMatrix("privacy", [
  {
    name: "screen",
    route: "/settings/privacy",
    setup: async (p) => {
      await p.evaluate(() => window.__ghiSettingsMock!.cloudRequests = 0);
    },
    ready: (p) => expect(p.getByRole("switch")).toBeVisible(),
  },
  {
    name: "lock-on",
    route: "/settings/privacy",
    ready: async (p) => {
      await p.getByRole("switch").click();
      await expect(p.getByRole("switch")).toBeChecked();
    },
  },
  {
    name: "export",
    route: "/settings/privacy",
    ready: async (p) => {
      await p.getByRole("button", { name: /^(Export everything|Xuất toàn bộ)/ }).click();
      await p.getByLabel(/^(Password|Mật khẩu)$/).fill("correct horse");
      await expect(p.getByRole("dialog")).toBeVisible();
    },
  },
  {
    name: "delete",
    route: "/settings/privacy",
    ready: async (p) => {
      await p.getByRole("button", { name: /^(Delete everything|Xoá toàn bộ)/ }).click();
      await expect(p.getByRole("dialog")).toBeVisible();
    },
  },
]);
