// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import { createMemoryHistory, RouterProvider } from "@tanstack/react-router";
import { I18nextProvider } from "react-i18next";
import { afterEach, describe, expect, it } from "vitest";
import { initMobileI18n } from "@ghi/i18n/mobile";
import { makeRouter } from "./router";

async function open(path: string, lng: "en" | "vi") {
  const router = makeRouter(createMemoryHistory({ initialEntries: [path] }));
  render(
    <I18nextProvider i18n={initMobileI18n(lng)}>
      <RouterProvider router={router} />
    </I18nextProvider>,
  );
  await router.load();
}

describe("tab shell", () => {
  afterEach(cleanup);

  it("opens on Meetings with the four tabs", async () => {
    await open("/", "en");
    for (const name of ["Meetings", "Record", "Search", "Settings"]) {
      expect(await screen.findByRole("link", { name })).toBeTruthy();
    }
    expect(await screen.findByText("Your meetings will appear here.")).toBeTruthy();
  });

  it("speaks Vietnamese", async () => {
    await open("/search", "vi");
    expect(await screen.findByRole("link", { name: "Cài đặt" })).toBeTruthy();
  });

  it("keeps onboarding outside the shell", async () => {
    await open("/onboarding", "en");
    expect(await screen.findByText(/Welcome to/)).toBeTruthy();
    expect(screen.queryByRole("navigation")).toBeNull();
  });
});
