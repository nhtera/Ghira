// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "../../bindings";
import { ipc } from "../../ipc";
import { AppLockCard } from "./app-lock-card";
import { renderSettings } from "./test-utils";

const serve = async (over: Partial<AppSettings>) => {
  const r = await ipc.commands.getSettings();
  if (r.status !== "ok") throw new Error("no settings");
  vi.spyOn(ipc.commands, "getSettings").mockResolvedValue({ status: "ok", data: { ...r.data, ...over } });
  return r.data;
};

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const toggle = () => screen.findByRole("switch", { name: "Lock Ghira with Touch ID" });

describe("app lock card", () => {
  it("turns on with the minutes and the prompt reason", async () => {
    const user = userEvent.setup();
    await serve({ appLock: false, lockAfterMinutes: 5 });
    const set = vi.spyOn(ipc.commands, "setAppLock").mockResolvedValue({ status: "ok", data: {} as AppSettings });
    renderSettings(<AppLockCard />);
    await user.click(await toggle());
    await waitFor(() => expect(set).toHaveBeenCalledWith(true, 5, "change the app lock"));
  });

  it("stays off, without an alarm, when the prompt is cancelled", async () => {
    const user = userEvent.setup();
    await serve({ appLock: false });
    vi.spyOn(ipc.commands, "setAppLock").mockResolvedValue({ status: "error", error: "notConfirmed" });
    renderSettings(<AppLockCard />);
    await user.click(await toggle());
    await waitFor(() => expect(ipc.commands.setAppLock).toHaveBeenCalled());
    expect((await toggle()).getAttribute("aria-checked")).toBe("false");
    expect(screen.queryByText(/not confirmed/)).toBeNull();
  });

  it("shows other errors", async () => {
    const user = userEvent.setup();
    await serve({ appLock: false });
    vi.spyOn(ipc.commands, "setAppLock").mockResolvedValue({ status: "error", error: "noAuthMethod" });
    renderSettings(<AppLockCard />);
    await user.click(await toggle());
    expect((await screen.findAllByText(/no password or Touch ID/)).length).toBeGreaterThan(0);
  });

  it("shows Lock now and the delay only when on, and locks", async () => {
    const user = userEvent.setup();
    await serve({ appLock: false });
    const off = renderSettings(<AppLockCard />);
    await toggle();
    expect(screen.queryByRole("button", { name: "Lock now" })).toBeNull();
    off.unmount();
    vi.restoreAllMocks();
    await serve({ appLock: true, lockAfterMinutes: 15 });
    const lock = vi.spyOn(ipc.commands, "lockNow").mockResolvedValue({ status: "ok", data: true });
    const set = vi.spyOn(ipc.commands, "setAppLock").mockResolvedValue({ status: "ok", data: {} as AppSettings });
    renderSettings(<AppLockCard />);
    await user.click(await screen.findByRole("button", { name: "Lock now" }));
    expect(lock).toHaveBeenCalledTimes(1);
    await user.selectOptions(screen.getByLabelText("Lock after"), "60");
    await waitFor(() => expect(set).toHaveBeenCalledWith(true, 60, "change the app lock"));
  });
});
