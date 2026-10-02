// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const commands = vi.hoisted(() => ({ voiceStatus: vi.fn(), listPeople: vi.fn(), deleteVoiceData: vi.fn(), enrollVoiceCancel: vi.fn() }));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { renderLive } from "../live/test-utils";
import { MyVoiceCard } from "./my-voice-card";

const people = { thirdParty: false, people: [{ gid: "me", name: "", isMe: true, colorSlot: 1, meetings: 5, openActions: 0, lastMetMs: null, voice: { kind: "self", atMs: 1 } }] };

beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset());
  commands.listPeople.mockReturnValue(ok(people));
  commands.deleteVoiceData.mockReturnValue(ok(null));
});
afterEach(cleanup);

describe("Settings → Your voice", () => {
  it("shows the stored profile and deletes it after a confirm", async () => {
    commands.voiceStatus.mockReturnValue(ok({ modelReady: true, meProfile: { atMs: Date.UTC(2026, 7, 1), samples: 6 }, enrolling: false }));
    renderLive(<MyVoiceCard />);
    expect(await screen.findByText(/Voice profile saved Aug 1, 2026 · 6 samples/)).toBeTruthy();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Delete my voice data…" }));
    expect(commands.deleteVoiceData).not.toHaveBeenCalled();
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Delete voice data" }));
    await waitFor(() => expect(commands.deleteVoiceData).toHaveBeenCalledExactlyOnceWith("me"));
  });

  it("no profile: says so, offers recording, no delete; without the model recording is off", async () => {
    commands.voiceStatus.mockReturnValue(ok({ modelReady: false, meProfile: null, enrolling: false }));
    renderLive(<MyVoiceCard />);
    expect(await screen.findByText(/No voice profile yet/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Delete my voice data…" })).toBeNull();
    expect((screen.getByRole("button", { name: "Record your voice again…" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/needs the voice model/)).toBeTruthy();
  });

  it("a failing read shows an error line instead of vanishing", async () => {
    commands.voiceStatus.mockReturnValue(Promise.resolve({ status: "error", error: "storage" }));
    renderLive(<MyVoiceCard />);
    expect((await screen.findByRole("alert")).textContent).toMatch(/Couldn.t read or write/);
  });
});
