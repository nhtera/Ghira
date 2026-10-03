// SPDX-License-Identifier: Apache-2.0
// The app-lock gate in the visual matrix (16-J).
import { expect } from "@playwright/test";
import { visualMatrix } from "./settings-matrix";

visualMatrix("lock", [
  {
    name: "gate",
    route: "/settings",
    ready: async (p) => {
      await p.evaluate(() => {
        window.__ghiSettingsMock!.faceIdOk = false;
        window.__ghiSettingsMock!.locked = true;
        window.dispatchEvent(new Event("focus"));
      });
      await expect(p.getByRole("dialog").getByRole("alert")).toBeVisible();
    },
  },
]);
