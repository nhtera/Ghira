// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SpeakerInfo } from "../../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const commands = vi.hoisted(() => ({
  knownSpeakerNames: vi.fn(),
  getSettings: vi.fn(),
  renameSpeaker: vi.fn(),
  mergeSpeakers: vi.fn(),
  splitSpeaker: vi.fn(),
  speakerNotAPerson: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { SpeakerChip } from "@ghi/ui";
import { renderLive, setLive } from "../live/test-utils";
import { SpeakerPopover } from "./speaker-popover";
import { matchNames } from "./name-field";

const sp = (id: number, over: Partial<SpeakerInfo> = {}): SpeakerInfo => ({ id, label: `Speaker ${id}`, colorSlot: id, isMe: false, provisional: false, notPerson: false, others: false, ...over });
const line = (gid: string, speaker: number, text: string) => ({ gid, speaker, t0Ms: 0, t1Ms: 1, text, overlap: false, words: [] });

function show(settings = { voiceProfilesThirdParty: false }) {
  commands.getSettings.mockImplementation(() => ok(settings));
  const speaker = sp(2);
  setLive({
    meeting: "m1",
    state: "recording",
    speakers: { 1: sp(1, { label: "Me", isMe: true }), 2: speaker, 3: sp(3, { label: "Minh" }) },
    lines: [line("a", 2, "first thing"), line("b", 2, "second thing"), line("c", 3, "third")],
  });
  renderLive(<SpeakerPopover speaker={speaker} chip={<SpeakerChip state="numbered" name="Speaker 2" colorSlot={2} />} />);
  return userEvent.setup();
}

beforeEach(() => {
  commands.knownSpeakerNames.mockImplementation(() => ok(["Linh", "Minh", "Sarah"]));
  for (const k of ["renameSpeaker", "mergeSpeakers", "speakerNotAPerson"] as const) commands[k].mockImplementation(() => ok(null));
  commands.splitSpeaker.mockImplementation(() => ok(9));
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const open = (user: ReturnType<typeof userEvent.setup>) => user.click(screen.getByRole("button", { name: /Speaker 2/ }));

describe("SpeakerPopover", () => {
  it("names a speaker in two steps: open, type, Enter", async () => {
    const user = show();
    await open(user);
    await user.keyboard("Hana{Enter}");
    await waitFor(() => expect(commands.renameSpeaker).toHaveBeenCalledWith(2, "Hana"));
    await waitFor(() => expect(screen.queryByRole("combobox")).toBeNull());
  });

  it("offers known people and takes the picked one", async () => {
    const user = show();
    await open(user);
    await user.keyboard("li");
    expect(await screen.findByRole("option", { name: "Linh" })).toBeTruthy();
    await user.keyboard("{ArrowDown}{Enter}");
    await waitFor(() => expect(commands.renameSpeaker).toHaveBeenCalledWith(2, "Linh"));
  });

  it("merges into another speaker", async () => {
    const user = show();
    await open(user);
    await user.click(screen.getByRole("button", { name: "Merge into…" }));
    await user.click(screen.getByRole("button", { name: /Minh/ }));
    await waitFor(() => expect(commands.mergeSpeakers).toHaveBeenCalledWith(2, 3));
  });

  it("marks as not a person", async () => {
    const user = show();
    await open(user);
    await user.click(screen.getByRole("button", { name: "Not a person (video, music)" }));
    await waitFor(() => expect(commands.speakerNotAPerson).toHaveBeenCalledWith(2));
  });

  it("splits the picked lines to a new speaker", async () => {
    const user = show();
    await open(user);
    await user.click(screen.getByRole("button", { name: "Split speaker…" }));
    expect((screen.getByRole("button", { name: /Move 0 lines/ }) as HTMLButtonElement).disabled).toBe(true);
    await user.click(screen.getByRole("checkbox", { name: "first thing" }));
    await user.click(screen.getByRole("checkbox", { name: "second thing" }));
    await user.click(screen.getByRole("button", { name: "Move 2 lines" }));
    await waitFor(() => expect(commands.splitSpeaker).toHaveBeenCalledWith(2, ["a", "b"]));
  });

  it("says so when nothing moved", async () => {
    commands.splitSpeaker.mockImplementation(() => ok(null));
    const user = show();
    await open(user);
    await user.click(screen.getByRole("button", { name: "Split speaker…" }));
    await user.click(screen.getByRole("checkbox", { name: "first thing" }));
    await user.click(screen.getByRole("button", { name: "Move 1 line" }));
    expect(await screen.findByText("No lines were moved.")).toBeTruthy();
  });

  it("shows a command failure", async () => {
    commands.renameSpeaker.mockImplementation(() => Promise.resolve({ status: "error" as const, error: "boom" }));
    const user = show();
    await open(user);
    await user.keyboard("Hana{Enter}");
    // A toast may render the message twice (title + live region): one is enough.
    expect((await screen.findAllByText(/boom/)).length).toBeGreaterThan(0);
  });

  it("does not offer saving the voice while third-party profiles are off", async () => {
    const user = show({ voiceProfilesThirdParty: false });
    await open(user);
    await screen.findByRole("combobox");
    expect(screen.queryByText("Save voice to their profile")).toBeNull();
  });

  it("with profiles on, saving the voice goes through the consent dialog first", async () => {
    const user = show({ voiceProfilesThirdParty: true });
    await open(user);
    await user.click(await screen.findByRole("checkbox", { name: "Save voice to their profile" }));
    await user.click(screen.getByRole("combobox"));
    await user.keyboard("Hana{Enter}");
    const dialog = await screen.findByRole("dialog", { name: /Save Hana’s voice\?/ });
    expect(commands.renameSpeaker).not.toHaveBeenCalled();
    await user.click(screen.getByRole("radio", { name: /Yes, Hana agreed/ }));
    await user.click(screen.getByRole("button", { name: "Save voice profile" }));
    await waitFor(() => expect(commands.renameSpeaker).toHaveBeenCalledWith(2, "Hana"));
    expect(dialog.isConnected).toBe(false);
  });

  it("Escape in the consent dialog names nobody", async () => {
    const user = show({ voiceProfilesThirdParty: true });
    await open(user);
    await user.click(await screen.findByRole("checkbox", { name: "Save voice to their profile" }));
    await user.click(screen.getByRole("combobox"));
    await user.keyboard("Hana{Enter}");
    await screen.findByRole("dialog");
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(commands.renameSpeaker).not.toHaveBeenCalled();
  });
});

it("matchNames filters and limits", () => {
  expect(matchNames(["Linh", "Minh", "Sarah"], "inh")).toEqual(["Linh", "Minh"]);
  expect(matchNames(["a", "b", "c"], "", 2)).toEqual(["a", "b"]);
});
