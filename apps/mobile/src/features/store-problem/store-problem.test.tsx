// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMemoryHistory, RouterProvider } from "@tanstack/react-router";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initMobileI18n } from "@ghi/i18n/mobile";
import type { StoreProblem } from "../../bindings";
import { makeRouter } from "../../router";
import { resetLockStore } from "../app-lock/lock-store";
import { AppErrorBoundary } from "./error-boundary";

const mock = () => window.__ghiSettingsMock!;
const PROBLEMS: Exclude<StoreProblem, "startup">[] = ["keyMissing", "keyLocked", "keystore", "damaged", "migration", "disk", "other"];
const ERASABLE: StoreProblem[] = ["keyMissing", "damaged"];

async function open(lng: "en" | "vi") {
  const router = makeRouter(createMemoryHistory({ initialEntries: ["/settings"] }));
  render(
    <I18nextProvider i18n={initMobileI18n(lng)}>
      <RouterProvider router={router} />
    </I18nextProvider>,
  );
}

describe("when the store can't be opened", () => {
  beforeEach(async () => {
    await import("../../ipc");
    mock().reset();
    resetLockStore();
  });
  afterEach(cleanup);

  it("says why in words, with the code, and offers both ways forward", async () => {
    mock().storeProblem = "keyMissing";
    await open("en");
    const gate = await screen.findByRole("dialog", { name: /can’t open your meetings/ });
    expect(gate.textContent).toContain("kept on this phone only");
    expect(gate.textContent).toContain("restoring a backup onto a new phone");
    expect(gate.textContent).not.toMatch(/iCloud/);
    expect(screen.getByTestId("store-problem-code").textContent).toBe("Error code: keyMissing");
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Start fresh" })).toBeTruthy();
  });

  it.each(PROBLEMS)("has English and Vietnamese words for %s", async (problem) => {
    for (const lng of ["en", "vi"] as const) {
      const t = initMobileI18n(lng).t;
      expect(t(`mobile.storeProblem.reason.${problem}`, { app: "Ghira" })).not.toMatch(/^mobile\./);
    }
  });

  it.each(PROBLEMS.filter((p) => !ERASABLE.includes(p)))("%s offers Try again but never Start fresh", async (problem) => {
    mock().storeProblem = problem;
    await open("en");
    await screen.findByRole("button", { name: "Try again" });
    expect(screen.queryByRole("button", { name: "Start fresh" })).toBeNull();
  });

  it("a store that opened but failed to start shows the generic screen, with no erase", async () => {
    mock().storeProblem = "startup";
    await open("en");
    expect(await screen.findByRole("dialog", { name: /couldn’t load/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Start fresh" })).toBeNull();
  });

  it("starts fresh only after the typed phrase", async () => {
    mock().storeProblem = "damaged";
    await open("en");
    fireEvent.click(await screen.findByRole("button", { name: "Start fresh" }));
    const erase = await screen.findByRole("button", { name: "Erase and start fresh" });
    expect((erase as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Type DELETE to confirm"), { target: { value: "delete" } });
    expect((erase as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(erase);
    await waitFor(() => expect(mock().wiped).toBe(true));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: /can’t open/ })).toBeNull());
  });

  it("asks for the Vietnamese phrase in Vietnamese", async () => {
    mock().storeProblem = "keyMissing";
    await open("vi");
    fireEvent.click(await screen.findByRole("button", { name: "Bắt đầu lại từ đầu" }));
    expect(await screen.findByLabelText("Gõ XOÁ để xác nhận")).toBeTruthy();
  });
});

describe("the error boundary", () => {
  afterEach(cleanup);

  it("shows \"couldn't load\" with Try again instead of a blank page", async () => {
    let broken = true;
    function Screen() {
      if (broken) throw new Error("boom");
      return <p>fine</p>;
    }
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    render(
      <I18nextProvider i18n={initMobileI18n("en")}>
        <AppErrorBoundary>
          <Screen />
        </AppErrorBoundary>
      </I18nextProvider>,
    );
    expect(screen.getByRole("alert").textContent).toContain("couldn’t load");
    // A retry that throws again says so.
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("It still didn’t load.")).toBeTruthy();
    broken = false;
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("fine")).toBeTruthy();
    vi.restoreAllMocks();
  });
});
