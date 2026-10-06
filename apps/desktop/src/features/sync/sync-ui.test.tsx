// SPDX-License-Identifier: Apache-2.0
// The Sync UI on the scripted mock core: pairing (code, expiry, regenerate,
// success), devices (unpair, wipe with the honest copy), the conflict banner,
// the mass-delete question and the delete messages.
import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ipc } from "../../ipc";
import { SyncSection } from "../settings/sync-section";
import { PrivacySection } from "../settings/privacy-section";
import { renderSettings } from "../settings/test-utils";
import { ConflictBanner } from "./conflict-banner";
import { announceDeleted } from "./delete-notice";
import { MassDeleteDialog, useMassDelete } from "./mass-delete-dialog";

type Hooks = {
  syncSet(s: string): void;
  syncSimulatePaired(): void;
  syncSimulatePairCodeSpent(): void;
  syncSimulateWipeDone(): void;
  syncSimulateProgress(n: number): void;
  syncSetDeleteEverywhere(s: string, w?: string[]): void;
  syncTransferNext(outcome: string): void;
};
const mock = () => (window as unknown as { __ghiMock: Hooks }).__ghiMock;

beforeEach(() => mock().syncSet("off"));
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  useMassDelete.setState({ ask: null });
});

describe("Settings > Sync", () => {
  it("is off until switched on, then offers pairing and says it stays local", async () => {
    const user = userEvent.setup();
    renderSettings(<SyncSection />);
    const sw = await screen.findByRole("switch", { name: "Sync with my phone" });
    expect(sw.getAttribute("aria-checked")).toBe("false");
    expect(screen.queryByRole("button", { name: "Pair a phone" })).toBeNull();
    await user.click(sw);
    expect(await screen.findByRole("button", { name: "Pair a phone" })).toBeTruthy();
    expect(screen.getByText("No phone paired yet.")).toBeTruthy();
    expect(screen.getByText(/Local network only/)).toBeTruthy();
    expect(screen.getByText("Can’t find your phone?")).toBeTruthy();
  });

  it("says voice profiles stay on each device and offers no switch for them", async () => {
    mock().syncSet("paired");
    renderSettings(<SyncSection />);
    expect(await screen.findByText("Voice profiles stay on each device.")).toBeTruthy();
    expect(screen.queryByText(/lets your phone name speakers/i)).toBeNull();
  });

  it("lists a paired phone and asks before unpairing", async () => {
    mock().syncSet("paired");
    const user = userEvent.setup();
    renderSettings(<SyncSection />);
    const row = await screen.findByTestId("device-device-iphone");
    expect(within(row).getByText("iPhone 16")).toBeTruthy();
    expect(within(row).getByText(/^Synced 4 min/)).toBeTruthy();
    const unpair = vi.spyOn(ipc.commands, "syncUnpair");
    await user.click(within(row).getByRole("button", { name: "Unpair" }));
    expect(unpair).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("dialog", { name: "Unpair iPhone 16?" });
    await user.click(within(dialog).getByRole("button", { name: "Unpair" }));
    await waitFor(() => expect(unpair).toHaveBeenCalledWith("device-iphone"));
    await screen.findByText("No phone paired yet.");
  });

  it("tells the truth about a wipe and then waits for the phone", async () => {
    mock().syncSet("paired");
    const user = userEvent.setup();
    renderSettings(<SyncSection />);
    const row = await screen.findByTestId("device-device-iphone");
    await user.click(within(row).getByRole("button", { name: "Unpair and wipe" }));
    const dialog = await screen.findByRole("dialog", { name: "Unpair iPhone 16 and wipe it?" });
    expect(within(dialog).getByText(/anyone who can unlock that phone/)).toBeTruthy();
    expect(within(dialog).getByText(/next time it is on your Wi-Fi/)).toBeTruthy();
    await user.click(within(dialog).getByRole("button", { name: "Unpair and wipe" }));
    expect(await screen.findByText("Waiting to wipe. Open Ghira on iPhone 16 to finish.")).toBeTruthy();
    expect(within(await screen.findByTestId("device-device-iphone")).queryByRole("button")).toBeNull();
    act(() => mock().syncSimulateWipeDone());
    await screen.findByText("No phone paired yet.");
    expect((await screen.findAllByText("iPhone 16 deleted its synced meetings.")).length).toBeGreaterThan(0);
  });

  it("shows the last failure in words and the pending work", async () => {
    mock().syncSet("error");
    renderSettings(<SyncSection />);
    expect((await screen.findByTestId("sync-error")).textContent).toMatch(/Couldn’t reach the other device/);
    expect(screen.getByText("Open Ghira on your phone to sync.")).toBeTruthy();
    expect(screen.getByText(/Last seen/)).toBeTruthy();
    expect(screen.getByText("Can’t find your phone?")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Export for another device" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("asks before applying another device's mass delete", async () => {
    mock().syncSet("paired");
    const user = userEvent.setup();
    renderSettings(
      <>
        <SyncSection />
        <MassDeleteDialog />
      </>,
    );
    await screen.findByTestId("device-device-iphone");
    mock().syncSet("needsConfirm");
    const dialog = await screen.findByRole("dialog", { name: "iPhone 16 deleted 12 meetings" });
    const confirm = vi.spyOn(ipc.commands, "syncConfirmMassDelete");
    await user.click(within(dialog).getByRole("button", { name: "Not now" }));
    await waitFor(() => expect(confirm).toHaveBeenCalledWith(false));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});

describe("export and import for another device", () => {
  const exportDialog = async (user: ReturnType<typeof userEvent.setup>) => {
    mock().syncSet("error");
    renderSettings(<SyncSection />);
    await user.click(await screen.findByRole("button", { name: "Export for another device" }));
    return screen.findByRole("dialog", { name: "Export for another device" });
  };

  it("asks for a repeated passphrase of at least 8 characters, then exports all meetings", async () => {
    const user = userEvent.setup();
    const run = vi.spyOn(ipc.commands, "syncExportForDevice");
    const dialog = await exportDialog(user);
    const save = within(dialog).getByRole("button", { name: "Save file…" }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    await user.type(within(dialog).getByLabelText("Passphrase"), "short");
    expect(save.disabled).toBe(true);
    expect(within(dialog).getByText("Use at least 8 characters.")).toBeTruthy();
    await user.clear(within(dialog).getByLabelText("Passphrase"));
    await user.type(within(dialog).getByLabelText("Passphrase"), "correct horse");
    await user.type(within(dialog).getByLabelText("Repeat the passphrase"), "correct hors");
    expect(save.disabled).toBe(true);
    await user.type(within(dialog).getByLabelText("Repeat the passphrase"), "e");
    expect(save.disabled).toBe(false);
    await user.click(save);
    // null: every finished meeting; the file's path never reaches the webview.
    await waitFor(() => expect(run).toHaveBeenCalledWith(null, "correct horse"));
    expect(await screen.findByText("Saved Ghira transfer 2026-10-06.ghix with 3 meetings")).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("stays open when the save dialog is cancelled", async () => {
    const user = userEvent.setup();
    const dialog = await exportDialog(user);
    mock().syncTransferNext("cancel");
    await user.type(within(dialog).getByLabelText("Passphrase"), "correct horse");
    await user.type(within(dialog).getByLabelText("Repeat the passphrase"), "correct horse");
    await user.click(within(dialog).getByRole("button", { name: "Save file…" }));
    await waitFor(() => expect((within(dialog).getByRole("button", { name: "Save file…" }) as HTMLButtonElement).disabled).toBe(false));
    expect(screen.getByRole("dialog", { name: "Export for another device" })).toBeTruthy();
  });

  it("imports with the passphrase, tells what was left out and refreshes the lists", async () => {
    const user = userEvent.setup();
    mock().syncSet("off");
    renderSettings(<SyncSection />);
    const run = vi.spyOn(ipc.commands, "syncImportFromDevice");
    await user.click(await screen.findByRole("button", { name: "Import from another device" }));
    const dialog = await screen.findByRole("dialog", { name: "Import from another device" });
    expect(within(dialog).queryByLabelText("Repeat the passphrase")).toBeNull();
    mock().syncTransferNext("refused");
    await user.type(within(dialog).getByLabelText("Passphrase"), "correct horse");
    await user.click(within(dialog).getByRole("button", { name: "Choose file…" }));
    await waitFor(() => expect(run).toHaveBeenCalledWith("correct horse", "Choose an export file"));
    expect(await screen.findByText("Imported 3 meetings from Ghira transfer 2026-10-06.ghix")).toBeTruthy();
    expect(await screen.findByText("1 meeting in the file was deleted here earlier, so it was left out.")).toBeTruthy();
  });

  it("words a wrong passphrase and a file that isn't an export, and keeps the sheet", async () => {
    const user = userEvent.setup();
    mock().syncSet("off");
    renderSettings(<SyncSection />);
    await user.click(await screen.findByRole("button", { name: "Import from another device" }));
    const dialog = await screen.findByRole("dialog", { name: "Import from another device" });
    await user.type(within(dialog).getByLabelText("Passphrase"), "correct horse");
    mock().syncTransferNext("wrongPassphrase");
    await user.click(within(dialog).getByRole("button", { name: "Choose file…" }));
    expect(await within(dialog).findByText(/doesn’t open this file/)).toBeTruthy();
    mock().syncTransferNext("notAnExport");
    await user.click(within(dialog).getByRole("button", { name: "Choose file…" }));
    expect(await within(dialog).findByText(/isn’t a file made with/)).toBeTruthy();
    expect(screen.getByRole("dialog", { name: "Import from another device" })).toBeTruthy();
  });
});

describe("pairing sheet", () => {
  it("shows the code with a countdown, expires, regenerates and ends on Paired", async () => {
    mock().syncSet("pairing");
    vi.useFakeTimers({ shouldAdvanceTime: true, toFake: ["setInterval", "clearInterval", "Date"] });
    renderSettings(<SyncSection />);
    const open = await screen.findByRole("button", { name: "Pair a phone" });
    await act(async () => open.click());
    const img = (await screen.findByAltText("Pairing code for your phone")) as HTMLImageElement;
    expect(img.src).toMatch(/^data:image\/svg\+xml/);
    expect(screen.getByTestId("pair-countdown").textContent).toMatch(/2:00|1:5\d/);
    await act(async () => vi.advanceTimersByTime(121_000));
    expect(await screen.findByText("This code has expired.")).toBeTruthy();
    expect(screen.queryByAltText("Pairing code for your phone")).toBeNull();
    await act(async () => screen.getByRole("button", { name: "Show a new code" }).click());
    expect(await screen.findByAltText("Pairing code for your phone")).toBeTruthy();
    act(() => mock().syncSimulatePaired());
    expect(await screen.findByRole("status", { name: "" })).toBeTruthy();
    expect(screen.getAllByText("Paired with iPhone 16").length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "Unpair" })).toBeTruthy();
  });

  it("asks for and shows a new code when failed tries used the old one up", async () => {
    mock().syncSet("pairing");
    const user = userEvent.setup();
    const open = vi.spyOn(ipc.commands, "syncPairOpen");
    renderSettings(<SyncSection />);
    await user.click(await screen.findByRole("button", { name: "Pair a phone" }));
    await screen.findByAltText("Pairing code for your phone");
    expect(open).toHaveBeenCalledTimes(1);
    act(() => mock().syncSimulatePairCodeSpent());
    await waitFor(() => expect(open).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("That code was used up by failed tries. Scan this new one.")).toBeTruthy();
    expect(await screen.findByAltText("Pairing code for your phone")).toBeTruthy();
  });

  it("closes the pairing listener when the sheet closes", async () => {
    mock().syncSet("pairing");
    const user = userEvent.setup();
    const close = vi.spyOn(ipc.commands, "syncPairClose");
    renderSettings(<SyncSection />);
    await user.click(await screen.findByRole("button", { name: "Pair a phone" }));
    await screen.findByAltText("Pairing code for your phone");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(close).toHaveBeenCalled());
  });

  it("refuses an offer that is not an SVG", async () => {
    mock().syncSet("pairing");
    const user = userEvent.setup();
    vi.spyOn(ipc.commands, "syncPairOpen").mockResolvedValue({ status: "ok", data: { qrSvg: "<script>1</script>", expiresMs: 120000 } });
    renderSettings(<SyncSection />);
    await user.click(await screen.findByRole("button", { name: "Pair a phone" }));
    expect(await screen.findByText("Couldn’t show a code. Try again.")).toBeTruthy();
  });
});

describe("conflict banner", () => {
  it("names the device, uses the copy and goes away", async () => {
    mock().syncSet("conflict");
    const user = userEvent.setup();
    const resolve = vi.spyOn(ipc.commands, "syncConflictResolve");
    renderSettings(<ConflictBanner meeting="m1" />);
    const banner = await screen.findByTestId("conflict-banner");
    expect(within(banner).getByText("Edited on iPhone 16")).toBeTruthy();
    await user.click(within(banner).getByRole("button", { name: "Use this" }));
    await waitFor(() => expect(resolve).toHaveBeenCalledWith("conflict-1", true));
    await waitFor(() => expect(screen.queryByTestId("conflict-banner")).toBeNull());
  });

  it("dismisses without using it, and renders nothing without a conflict", async () => {
    mock().syncSet("conflict");
    const user = userEvent.setup();
    const resolve = vi.spyOn(ipc.commands, "syncConflictResolve");
    renderSettings(<ConflictBanner meeting="m1" />);
    await user.click(await screen.findByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(resolve).toHaveBeenCalledWith("conflict-1", false));
    cleanup();
    mock().syncSet("paired");
    renderSettings(<ConflictBanner meeting="m1" />);
    await waitFor(() => expect(screen.queryByTestId("conflict-banner")).toBeNull());
  });
});

describe("delete messages", () => {
  it("says nothing when sync is off", async () => {
    const show = vi.fn();
    await announceDeleted(show, ((k: string) => k) as never);
    expect(show).not.toHaveBeenCalled();
  });

  it("says it is deleted here, then on all devices once a sync has nothing pending", async () => {
    mock().syncSet("paired");
    const show = vi.fn();
    const t = ((k: string, o?: { device?: string }) => `${k}|${o?.device ?? ""}`) as never;
    await announceDeleted(show, t);
    expect(show).toHaveBeenCalledWith({ title: "settings.sync.deletedHere|iPhone 16" });
    mock().syncSimulateProgress(3);
    expect(show).toHaveBeenCalledTimes(1);
    mock().syncSimulateProgress(0);
    expect(show).toHaveBeenLastCalledWith({ tone: "success", title: "settings.sync.deletedEverywhere|" });
    mock().syncSimulateProgress(0);
    expect(show).toHaveBeenCalledTimes(2);
  });
});

describe("delete everything with paired devices", () => {
  const typeWord = async (user: ReturnType<typeof userEvent.setup>) => {
    await user.click(await screen.findByRole("button", { name: "Delete all meetings and voice data…" }));
    await user.type(screen.getByLabelText("Type DELETE to confirm"), "DELETE{Enter}");
  };

  it("asks first, and Delete here only deletes without a wipe", async () => {
    mock().syncSet("paired");
    const user = userEvent.setup();
    const del = vi.spyOn(ipc.commands, "deleteAllData");
    const off = vi.spyOn(ipc.commands, "syncSetEnabled");
    renderSettings(<PrivacySection />);
    await waitFor(() => expect(screen.getByTestId("strict-offline-local-note")).toBeTruthy());
    await typeWord(user);
    expect(await screen.findByText("Also delete on your paired devices?")).toBeTruthy();
    expect(del).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Delete here only" }));
    await waitFor(() => expect(del).toHaveBeenCalledTimes(1));
    expect(del).toHaveBeenCalledWith(false);
    expect(off).not.toHaveBeenCalled();
  });

  it("Delete everywhere waits for a device that is out of reach", async () => {
    mock().syncSet("paired");
    const user = userEvent.setup();
    const del = vi.spyOn(ipc.commands, "deleteAllData").mockImplementation(() => new Promise(() => {}));
    renderSettings(<PrivacySection />);
    await waitFor(() => expect(screen.getByTestId("strict-offline-local-note")).toBeTruthy());
    await typeWord(user);
    const skip = vi.spyOn(ipc.commands, "syncDeleteEverywhereSkip");
    await user.click(await screen.findByRole("button", { name: "Delete everywhere" }));
    expect(del).toHaveBeenCalledTimes(1);
    expect(del).toHaveBeenCalledWith(true);
    mock().syncSetDeleteEverywhere("waiting", ["iPhone 16"]);
    const waiting = await screen.findByTestId("delete-waiting", {}, { timeout: 3000 });
    expect(within(waiting).getByText("Waiting for iPhone 16…")).toBeTruthy();
    // Skipping ends the wait in the core; the delete already running goes on (no second call).
    await user.click(within(waiting).getByRole("button", { name: "Delete here only" }));
    await waitFor(() => expect(skip).toHaveBeenCalledTimes(1));
    expect(del).toHaveBeenCalledTimes(1);
  });

  it("does not ask when nobody is paired", async () => {
    const user = userEvent.setup();
    const del = vi.spyOn(ipc.commands, "deleteAllData");
    renderSettings(<PrivacySection />);
    await typeWord(user);
    await waitFor(() => expect(del).toHaveBeenCalledTimes(1));
    expect(screen.queryByText("Also delete on your paired devices?")).toBeNull();
  });
});
