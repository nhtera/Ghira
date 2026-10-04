// SPDX-License-Identifier: Apache-2.0
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MeetingSpeaker, SegmentView } from "../../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: string) => Promise.resolve({ status: "error" as const, error });
const commands = vi.hoisted(() => ({
  renameMeetingSpeaker: vi.fn(),
  setSpeakerMe: vi.fn(),
  clearSpeakerMe: vi.fn(),
  mergeMeetingSpeakers: vi.fn(),
  splitMeetingSpeaker: vi.fn(),
  setSpeakerNotPerson: vi.fn(),
  issueAudioSample: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands, audioUrl: (t: string) => `ghi-audio://${t}` } }));

import { ToastProvider } from "@ghi/ui";
import { SpeakerPanel, type SpeakerPanelProps } from "./speaker-panel";

const sp = (gid: string, number: number, over: Partial<MeetingSpeaker> = {}): MeetingSpeaker => ({
  gid, name: null, number, colorSlot: number, isMe: false, notPerson: false, lines: 3, sampleT0Ms: null, sampleT1Ms: null, suggestion: null, ...over,
});
const seg = (gid: string, speakerGid: string, t0Ms: number, text: string): SegmentView => ({
  gid, speakerGid, t0Ms, t1Ms: t0Ms + 1000, text, language: null, confidence: null, edited: false, overlap: false, words: [],
});

const speakers = [sp("a", 1, { name: "Linh" }), sp("b", 2, { name: "Minh" }), sp("c", 3)];
const segments = [seg("l1", "a", 1000, "First line"), seg("l2", "a", 5000, "Second line"), seg("l3", "a", 9000, "Third line"), seg("x1", "b", 3000, "Other")];

let onClose: () => void;
let onMerged: () => void;
const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
const tree = (over: Partial<SpeakerPanelProps> = {}) => (
  <QueryClientProvider client={client}>
    <ToastProvider label="notifications">
      <SpeakerPanel meeting="m1" mode="room" speaker={speakers[0]!} speakers={speakers} segments={segments} fromSegment="l2" onClose={onClose} onMerged={onMerged} {...over} />
    </ToastProvider>
  </QueryClientProvider>
);
const show = (over: Partial<SpeakerPanelProps> = {}) => {
  onClose = vi.fn<() => void>();
  onMerged = vi.fn<() => void>();
  const r = render(tree(over));
  return { ...r, rerender: (next: Partial<SpeakerPanelProps>) => r.rerender(tree(next)) };
};

beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset().mockReturnValue(ok(null)));
  commands.splitMeetingSpeaker.mockReturnValue(ok("new"));
});
afterEach(cleanup);

describe("SpeakerPanel", () => {
  it("is a dialog named by the speaker, with the name field focused, and Escape closes it", async () => {
    show();
    expect(screen.getByRole("dialog", { name: "Linh" })).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "Rename speaker" }));
    await userEvent.setup().keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("keeps Tab inside the panel", async () => {
    show();
    const user = userEvent.setup();
    const dialog = screen.getByRole("dialog");
    for (let i = 0; i < 30; i++) {
      await user.tab();
      expect(dialog.contains(document.activeElement)).toBe(true);
    }
    await user.tab({ shift: true });
    expect(dialog.contains(document.activeElement)).toBe(true);
  });

  it("renames; an empty name goes back to Speaker N", async () => {
    show();
    const user = userEvent.setup();
    const box = screen.getByRole("textbox", { name: "Rename speaker" });
    await user.clear(box);
    await user.type(box, "Lan{Enter}");
    await waitFor(() => expect(commands.renameMeetingSpeaker).toHaveBeenCalledExactlyOnceWith("m1", "a", "Lan"));
    expect(await screen.findByText("Renamed to Lan on every line")).toBeTruthy();
    await user.clear(box);
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(commands.renameMeetingSpeaker).toHaveBeenLastCalledWith("m1", "a", ""));
  });

  it("Save is off until the name changes", () => {
    show();
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("marks and clears Me; a call has no Me buttons", async () => {
    const { unmount } = show();
    await userEvent.setup().click(screen.getByRole("button", { name: "This is me" }));
    await waitFor(() => expect(commands.setSpeakerMe).toHaveBeenCalledExactlyOnceWith("m1", "a"));
    unmount();
    show({ speaker: sp("a", 1, { name: "Linh", isMe: true }) });
    await userEvent.setup().click(screen.getByRole("button", { name: "Not me" }));
    await waitFor(() => expect(commands.clearSpeakerMe).toHaveBeenCalledExactlyOnceWith("m1", "a"));
    cleanup();
    show({ mode: "call" });
    expect(screen.queryByRole("button", { name: "This is me" })).toBeNull();
  });

  it("merge asks first, naming both and the lines; Cancel changes nothing", async () => {
    show();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Minh/ }));
    const ask = screen.getByRole("alertdialog");
    expect(ask.textContent).toContain("Merge Linh into Minh? All 3 lines move to Minh. There is no undo.");
    await user.click(within(ask).getByRole("button", { name: "Cancel" }));
    expect(commands.mergeMeetingSpeakers).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    await user.click(screen.getByRole("button", { name: /Minh/ }));
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Merge" }));
    await waitFor(() => expect(commands.mergeMeetingSpeakers).toHaveBeenCalledExactlyOnceWith("m1", "a", "b"));
    expect(onMerged).toHaveBeenCalled();
  });

  it("Escape in the merge question cancels it, not the panel", async () => {
    show();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Minh/ }));
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("splits from the line it was opened from", async () => {
    show();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Split speaker…" }));
    expect(screen.getByRole("radio", { name: "From 00:05 on (2 lines)" })).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Move 2 lines" }));
    await waitFor(() => expect(commands.splitMeetingSpeaker).toHaveBeenCalledExactlyOnceWith("m1", "a", [], "l2"));
    expect(await screen.findByText("Moved 2 lines to a new speaker")).toBeTruthy();
  });

  it("splits the picked lines, and never all of them", async () => {
    show({ fromSegment: null });
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Split speaker…" }));
    const move = screen.getByRole("button", { name: "Move 0 lines" }) as HTMLButtonElement;
    expect(move.disabled).toBe(true);
    await user.click(screen.getByRole("checkbox", { name: /Third line/ }));
    expect((screen.getByRole("button", { name: "Move 1 line" }) as HTMLButtonElement).disabled).toBe(false);
    await user.click(screen.getByRole("checkbox", { name: /First line/ }));
    await user.click(screen.getByRole("checkbox", { name: /Second line/ }));
    expect((screen.getByRole("button", { name: "Move 3 lines" }) as HTMLButtonElement).disabled).toBe(true);
    await user.click(screen.getByRole("checkbox", { name: /Second line/ }));
    await user.click(screen.getByRole("button", { name: "Move 2 lines" }));
    await waitFor(() => expect(commands.splitMeetingSpeaker).toHaveBeenCalledExactlyOnceWith("m1", "a", ["l3", "l1"], null));
  });

  it("not a person asks first, then marks; a marked speaker can be restored", async () => {
    show();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Not a person (video, music)" }));
    expect(screen.getByRole("alertdialog").textContent).toContain("Mark Linh as not a person?");
    expect(commands.setSpeakerNotPerson).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Mark as not a person" }));
    await waitFor(() => expect(commands.setSpeakerNotPerson).toHaveBeenCalledExactlyOnceWith("m1", "a", true));
    cleanup();
    show({ speaker: sp("a", 1, { name: "Linh", notPerson: true }) });
    await userEvent.setup().click(screen.getByRole("button", { name: "This is a person" }));
    await waitFor(() => expect(commands.setSpeakerNotPerson).toHaveBeenLastCalledWith("m1", "a", false));
  });

  it("plays the sample", async () => {
    commands.issueAudioSample.mockReturnValue(ok("tok"));
    show({ speaker: sp("a", 1, { name: "Linh", sampleT0Ms: 1000, sampleT1Ms: 4000 }) });
    await userEvent.setup().click(screen.getByRole("button", { name: "Play 3 s sample" }));
    await waitFor(() => expect(commands.issueAudioSample).toHaveBeenCalledWith("m1", 1000, 4000, null));
    expect(await screen.findByLabelText("Playing a 3 s sample")).toBeTruthy();
  });

  const sentences: [string, () => Promise<unknown>, string][] = [
    ["liveMeeting", () => userEvent.setup().click(screen.getByRole("button", { name: "This is me" })), "This meeting is being recorded"],
    ["notASpeaker", () => userEvent.setup().click(screen.getByRole("button", { name: "This is me" })), "no longer in this meeting"],
    ["isMe", () => notPerson(), "Me can’t be marked as not a person"],
    ["farSide", () => merge(), "only your own microphone can be Me"],
    ["sameSpeaker", () => merge(), "can’t be merged into themselves"],
    ["storage", () => merge(), "Couldn’t read or write the meeting"],
    ["somethingNew", () => merge(), "That didn’t work: somethingNew"],
  ];
  const merge = async () => {
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Minh/ }));
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Merge" }));
  };
  const notPerson = async () => {
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Not a person (video, music)" }));
    await user.click(screen.getByRole("button", { name: "Mark as not a person" }));
  };
  it.each(sentences)("says %s as a sentence, in the panel", async (code, act, text) => {
    for (const c of [commands.setSpeakerMe, commands.mergeMeetingSpeakers, commands.setSpeakerNotPerson]) c.mockReturnValue(err(code));
    show();
    await act();
    const alert = await screen.findByTestId("speaker-panel-error");
    expect(alert.textContent).toContain(text);
    expect(alert.getAttribute("role")).toBe("alert");
    expect(onMerged).not.toHaveBeenCalled();
  });

  it("says nothingToSplit and wholeSpeaker when the core refuses a split", async () => {
    const user = userEvent.setup();
    show();
    await user.click(screen.getByRole("button", { name: "Split speaker…" }));
    for (const [code, text] of [["nothingToSplit", "Pick some of this speaker’s lines"], ["wholeSpeaker", "Leave at least one line"]] as const) {
      commands.splitMeetingSpeaker.mockReturnValue(err(code));
      await user.click(screen.getByRole("button", { name: "Move 2 lines" }));
      expect((await screen.findByTestId("speaker-panel-error")).textContent).toContain(text);
    }
  });

  it("shows the speaker as an initial on a color plus the name, never color alone", () => {
    show({ speaker: speakers[2]! });
    const dialog = screen.getByRole("dialog", { name: "Speaker 3" });
    expect(within(dialog).getByText("3")).toBeTruthy();
    expect(within(dialog).getByText("Speaker · Speaker 3")).toBeTruthy();
  });

  it("Shift+Tab from the first control wraps to the last", async () => {
    show();
    const user = userEvent.setup();
    const dialog = screen.getByRole("dialog");
    const items = [...dialog.querySelectorAll<HTMLElement>('button:not([disabled]), input:not([disabled])')];
    items[0]!.focus();
    await user.tab({ shift: true });
    expect(document.activeElement).toBe(items.at(-1));
  });

  it("focus pulled back after a click on the panel's padding: Escape and Tab still work", async () => {
    show();
    const user = userEvent.setup();
    const dialog = screen.getByRole("dialog");
    await user.click(dialog);
    expect(document.activeElement).toBe(dialog);
    await user.tab();
    expect(dialog.contains(document.activeElement) && document.activeElement !== dialog).toBe(true);
    await user.click(dialog);
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("focus that leaves the panel comes back to it", () => {
    show();
    const dialog = screen.getByRole("dialog");
    (document.activeElement as HTMLElement).blur();
    fireEvent.focusOut(dialog.querySelector("input")!, { relatedTarget: null });
    expect(document.activeElement).toBe(dialog);
  });

  it("a new speaker starts clean (the panel is keyed by speaker in the transcript)", async () => {
    const user = userEvent.setup();
    const { rerender } = show();
    const box = screen.getByRole("textbox", { name: "Rename speaker" });
    await user.clear(box);
    await user.type(box, "Typed for Linh");
    await user.click(screen.getByRole("button", { name: /Minh/ }));
    expect(screen.getByRole("alertdialog")).toBeTruthy();
    rerender({ speaker: speakers[1]!, fromSegment: "x1", key: "b:x1" } as Partial<SpeakerPanelProps>);
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect((screen.getByRole("textbox", { name: "Rename speaker" }) as HTMLInputElement).value).toBe("Minh");
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("while a request runs, merge chips, split and not-a-person are off (no double submit)", async () => {
    let release: (v: unknown) => void = () => {};
    commands.mergeMeetingSpeakers.mockReturnValue(new Promise((r) => (release = r)));
    show();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Minh/ }));
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Merge" }));
    // The chips are off through their fieldset (:disabled, not the property).
    for (const name of [/Minh/, /Speaker 3/]) expect(screen.getByRole("button", { name }).closest("fieldset")?.disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Split speaker…" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Not a person (video, music)" }) as HTMLButtonElement).disabled).toBe(true);
    release({ status: "ok", data: null });
    await waitFor(() => expect(onMerged).toHaveBeenCalled());
    expect(commands.mergeMeetingSpeakers).toHaveBeenCalledTimes(1);
  });

  it("a split from a line leaves a valid split choice behind", async () => {
    const user = userEvent.setup();
    const { rerender } = show();
    await user.click(screen.getByRole("button", { name: "Split speaker…" }));
    await user.click(screen.getByRole("button", { name: "Move 2 lines" }));
    await waitFor(() => expect(commands.splitMeetingSpeaker).toHaveBeenCalled());
    // The reload: the line it started from now belongs to the new speaker.
    const left = segments.map((g) => (g.gid === "l2" || g.gid === "l3" ? { ...g, speakerGid: "new" } : g));
    rerender({ segments: left });
    await user.click(screen.getByRole("button", { name: "Split speaker…" }));
    expect(screen.queryByRole("radio", { name: /^From/ })).toBeNull();
    expect((screen.getByRole("radio", { name: "These lines" }) as HTMLInputElement).checked).toBe(true);
  });

  it("codes the panel has no sentence for fall back to People's (notMe)", async () => {
    commands.clearSpeakerMe.mockReturnValue(err("notMe"));
    show({ speaker: sp("a", 1, { name: "Linh", isMe: true }) });
    await userEvent.setup().click(screen.getByRole("button", { name: "Not me" }));
    expect((await screen.findByTestId("speaker-panel-error")).textContent).toContain("This speaker isn’t marked as Me.");
  });
});
