// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { UpdateStatus } from "../../bindings";
import { ipc } from "../../ipc";
import { renderSettings } from "./test-utils";
import { UpdateBanner } from "./update-banner";
import { UpdatesCard } from "./updates-card";

const base: UpdateStatus = { configured: true, checking: false, lastCheck: Date.now() - 60_000, available: null, notesUrl: null, ready: false, runningPulled: false, reinstallNeeded: false, error: null };
const serve = (over: Partial<UpdateStatus>) => vi.spyOn(ipc.commands, "updateStatus").mockResolvedValue({ ...base, ...over });

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("updates card", () => {
  it("says it is up to date and checks on demand", async () => {
    const user = userEvent.setup();
    serve({});
    const check = vi.spyOn(ipc.commands, "checkForUpdates").mockResolvedValue({ status: "ok", data: { ...base, available: "9.9.9", ready: true } });
    renderSettings(<UpdatesCard />);
    expect((await screen.findByTestId("update-line")).textContent).toContain("up to date");
    await user.click(screen.getByRole("button", { name: "Check for updates" }));
    await waitFor(() => expect(screen.getByTestId("update-line").textContent).toBe("Version 9.9.9 is ready."));
    expect(check).toHaveBeenCalledTimes(1);
  });

  it("asks before it restarts to install", async () => {
    const user = userEvent.setup();
    serve({ available: "9.9.9", ready: true });
    const install = vi.spyOn(ipc.commands, "installUpdate").mockResolvedValue({ status: "ok", data: null });
    renderSettings(<UpdatesCard />);
    await user.click(await screen.findByRole("button", { name: "Restart to update" }));
    expect(install).not.toHaveBeenCalled();
    const confirm = screen.getByRole("alertdialog");
    expect(confirm.textContent).toContain("9.9.9");
    await user.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(install).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Restart to update" }));
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Restart and update" }));
    await waitFor(() => expect(install).toHaveBeenCalledTimes(1));
  });

  it("is disabled and explained when the build can't update itself", async () => {
    serve({ configured: false });
    renderSettings(<UpdatesCard />);
    expect(await screen.findByText("This build doesn’t update itself.")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Check for updates" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("is disabled under strict offline", async () => {
    serve({});
    vi.spyOn(ipc.commands, "getSettings").mockResolvedValue({ status: "ok", data: { ...(await ipc.commands.getSettings().then((r) => (r.status === "ok" ? r.data : ({} as never)))), strictOffline: true } });
    renderSettings(<UpdatesCard />);
    expect(await screen.findByText(/Strict offline is on/)).toBeTruthy();
    expect((screen.getByRole("button", { name: "Check for updates" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows an error from the check", async () => {
    serve({ error: "signature check failed" });
    renderSettings(<UpdatesCard />);
    expect(await screen.findByText("signature check failed")).toBeTruthy();
  });
});

describe("update banner", () => {
  it("renders nothing when the running version is fine", async () => {
    const s = serve({ available: "9.9.9", ready: true });
    renderSettings(<UpdateBanner />);
    await waitFor(() => expect(s).toHaveBeenCalled());
    expect(screen.queryByTestId("update-banner")).toBeNull();
  });

  it("withdrawn and ready: restart, after which install runs", async () => {
    const user = userEvent.setup();
    serve({ runningPulled: true, available: "9.9.9", ready: true });
    const install = vi.spyOn(ipc.commands, "installUpdate").mockResolvedValue({ status: "ok", data: null });
    renderSettings(<UpdateBanner />);
    expect((await screen.findByTestId("update-banner")).textContent).toContain("withdrawn. Restart to update.");
    await user.click(screen.getByRole("button", { name: "Restart to update" }));
    expect(install).toHaveBeenCalledTimes(1);
  });

  it("withdrawn and not ready: points to Settings → About", async () => {
    const user = userEvent.setup();
    serve({ runningPulled: true });
    const open = vi.fn();
    renderSettings(<UpdateBanner onOpenAbout={open} />);
    expect((await screen.findByTestId("update-banner")).textContent).toContain("Settings → About");
    await user.click(screen.getByRole("button", { name: "Open Settings → About" }));
    expect(open).toHaveBeenCalled();
  });

  it("reinstall needed: tells to download, no button", async () => {
    serve({ reinstallNeeded: true });
    renderSettings(<UpdateBanner />);
    const b = await screen.findByTestId("update-banner");
    expect(b.textContent).toContain("Download the new version from the website");
    expect(within(b).queryByRole("button")).toBeNull();
  });
});
