// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const commands = vi.hoisted(() => ({ setMeetingSensitive: vi.fn(), sensitiveNext: vi.fn(), setSensitiveNext: vi.fn() }));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { renderLive, setLive } from "../live/test-utils";
import { LiveHeader } from "../live/live-header";
import { SensitiveBadge, SensitiveConfirm, SensitiveStartToggle } from ".";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("sensitive mode", () => {
  it("the badge says what is (not) kept, in words", () => {
    renderLive(<SensitiveBadge />);
    expect(screen.getByTestId("sensitive-badge").textContent).toBe("Sensitive · no audio kept");
  });

  it("arms the next recording in the core with a switch", async () => {
    let armed = false;
    commands.sensitiveNext.mockImplementation(() => ok(armed));
    commands.setSensitiveNext.mockImplementation((on: boolean) => {
      armed = on;
      return ok(null);
    });
    renderLive(<SensitiveStartToggle />);
    const sw = await screen.findByRole("switch", { name: "Sensitive" });
    expect(sw.getAttribute("aria-checked")).toBe("false");
    await userEvent.click(sw);
    expect(commands.setSensitiveNext).toHaveBeenLastCalledWith(true);
    await waitFor(() => expect(sw.getAttribute("aria-checked")).toBe("true"));
    await userEvent.click(sw);
    expect(commands.setSensitiveNext).toHaveBeenLastCalledWith(false);
    await waitFor(() => expect(sw.getAttribute("aria-checked")).toBe("false"));
  });

  it("asks before deleting audio, says what happens, then turns it on", async () => {
    commands.setMeetingSensitive.mockImplementation(() => ok(null));
    const onClose = vi.fn();
    renderLive(<SensitiveConfirm meeting="m1" recording={false} onClose={onClose} />);
    expect(screen.getByText(/Its audio is deleted now and only the transcript stays/)).toBeTruthy();
    expect(commands.setMeetingSensitive).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Make sensitive" }));
    await waitFor(() => expect(commands.setMeetingSensitive).toHaveBeenCalledWith("m1", true));
    expect(onClose).toHaveBeenCalled();
  });

  it("while recording it warns that it cannot be turned off, and Cancel changes nothing", async () => {
    const onClose = vi.fn();
    renderLive(<SensitiveConfirm meeting="m1" recording onClose={onClose} />);
    expect(screen.getByText(/can't be turned off during this recording/)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(commands.setMeetingSensitive).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("the live header offers it in More, and a sensitive recording shows the badge and cannot leave", async () => {
    const onSensitive = vi.fn();
    const header = () => (
      <LiveHeader meeting="m1" mode="room" layout="transcript" onLayout={() => {}} onDiscard={() => {}} onSensitive={onSensitive} discardSeconds={[60]} compact={false} />
    );
    setLive({ state: "recording", meeting: "m1", session: { mode: "room", language: null, title: "", consentConfirmed: false, sensitive: false } });
    const { unmount } = renderLive(header());
    expect(screen.queryByTestId("sensitive-badge")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "More actions" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: /Sensitive meeting…/ }));
    expect(onSensitive).toHaveBeenCalled();
    unmount();

    setLive({ state: "recording", meeting: "m1", session: { mode: "room", language: null, title: "", consentConfirmed: false, sensitive: true } });
    renderLive(header());
    expect(screen.getByTestId("sensitive-badge")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "More actions" }));
    const item = await screen.findByRole("menuitem", { name: /Sensitive meeting/ });
    expect(item.getAttribute("aria-disabled") ?? item.getAttribute("data-disabled")).not.toBeNull();
  });
});
