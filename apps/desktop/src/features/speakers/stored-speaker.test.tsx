// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MeetingSpeaker } from "../../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: string) => Promise.resolve({ status: "error" as const, error });
const commands = vi.hoisted(() => ({
  setSpeakerMe: vi.fn(),
  clearSpeakerMe: vi.fn(),
  acceptVoiceSuggestion: vi.fn(),
  dismissVoiceSuggestion: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { renderLive } from "../live/test-utils";
import { StoredSpeaker } from "./stored-speaker";

const speaker = (over: Partial<MeetingSpeaker> = {}): MeetingSpeaker => ({
  gid: "s2",
  name: null,
  number: 2,
  colorSlot: 3,
  isMe: false,
  notPerson: false,
  lines: 4,
  sampleT0Ms: null,
  sampleT1Ms: null,
  suggestion: null,
  ...over,
});
const meSuggestion = { personGid: "me", name: "", isMe: true, score: 0.74 };
const show = (s: MeetingSpeaker, mode = "room") => renderLive(<StoredSpeaker meeting="m1" mode={mode} speaker={s} />);

beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset().mockReturnValue(ok(null)));
});
afterEach(cleanup);

describe("StoredSpeaker voice actions", () => {
  it("a Me suggestion shows 'Sounds like Me' and accepting it marks Me", async () => {
    show(speaker({ suggestion: meSuggestion }));
    const chip = screen.getByTestId("voice-suggestion");
    expect(chip.textContent).toContain("Sounds like Me");
    await userEvent.setup().click(screen.getByRole("button", { name: "Accept Me as this speaker" }));
    await waitFor(() => expect(commands.acceptVoiceSuggestion).toHaveBeenCalledExactlyOnceWith("m1", "s2"));
    expect(await screen.findByText("Marked as Me")).toBeTruthy();
  });

  it("dismissing a suggestion calls only dismiss", async () => {
    show(speaker({ suggestion: meSuggestion }));
    await userEvent.setup().click(screen.getByRole("button", { name: "Dismiss the suggestion for Speaker 2" }));
    await waitFor(() => expect(commands.dismissVoiceSuggestion).toHaveBeenCalledExactlyOnceWith("m1", "s2"));
    expect(commands.acceptVoiceSuggestion).not.toHaveBeenCalled();
  });

  it("no suggestion, no chip", () => {
    show(speaker());
    expect(screen.queryByTestId("voice-suggestion")).toBeNull();
  });

  it("This is me marks the speaker; Not me clears it", async () => {
    const user = userEvent.setup();
    const first = show(speaker());
    await user.click(screen.getByRole("button", { name: /Speaker 2/ }));
    await user.click(await screen.findByRole("button", { name: "This is me" }));
    await waitFor(() => expect(commands.setSpeakerMe).toHaveBeenCalledExactlyOnceWith("m1", "s2"));
    first.unmount();

    show(speaker({ isMe: true }));
    await user.click(screen.getByRole("button", { name: /Me/ }));
    expect(screen.queryByRole("button", { name: "This is me" })).toBeNull();
    await user.click(await screen.findByRole("button", { name: "Not me" }));
    await waitFor(() => expect(commands.clearSpeakerMe).toHaveBeenCalledExactlyOnceWith("m1", "s2"));
  });

  it("in a call there is no This is me / Not me, only the chip", async () => {
    show(speaker(), "call");
    expect(screen.queryByRole("button", { name: /Speaker 2/ })).toBeNull();
    expect(screen.getByText("Speaker 2")).toBeTruthy();
    cleanup();
    show(speaker({ isMe: true }), "call");
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("a farSide refusal is shown as a sentence, not a raw code", async () => {
    commands.setSpeakerMe.mockReturnValue(err("farSide"));
    const user = userEvent.setup();
    show(speaker());
    await user.click(screen.getByRole("button", { name: /Speaker 2/ }));
    await user.click(await screen.findByRole("button", { name: "This is me" }));
    expect(await screen.findByText("In a call only your own microphone can be Me.")).toBeTruthy();
  });
});
