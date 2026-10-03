// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  MeetingDetail,
  MeetingNotes,
  NoteBlockView,
} from "../../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const commands = vi.hoisted(() => ({
  meetingNotes: vi.fn(),
  updateNoteBlock: vi.fn(),
  addNoteBlock: vi.fn(),
  deleteNoteBlock: vi.fn(),
  setActionDone: vi.fn(),
  setActionOwner: vi.fn(),
  addActionItem: vi.fn(),
  deleteActionItem: vi.fn(),
  updateActionItem: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { renderLive } from "../live/test-utils";
import { usePlayer } from "../../state/player";
import { CitationLink } from "../citation/citation-link";
import { NotesTab } from "./notes-tab";

const block = (
  gid: string,
  kind: string,
  origin: NoteBlockView["origin"],
  text: string,
  citations: NoteBlockView["citations"] = [],
): NoteBlockView => ({ gid, kind, origin, text, pinned: false, citations });
const cite = {
  t0Ms: 12_000,
  t1Ms: 15_000,
  quote: "we ship on the 12th",
  speakerGid: "s1",
  stale: false,
  missing: false,
};

const detail = {
  gid: "m1",
  audioAvailable: true,
  template: null,
  speakers: [
    {
      gid: "s1",
      name: "Sarah",
      number: 1,
      colorSlot: 2,
      isMe: false,
      notPerson: false,
      lines: 3,
      sampleT0Ms: null,
      sampleT1Ms: null,
    },
    {
      gid: "s2",
      name: null,
      number: 2,
      colorSlot: 3,
      isMe: false,
      notPerson: false,
      lines: 1,
      sampleT0Ms: null,
      sampleT1Ms: null,
    },
  ],
} as unknown as MeetingDetail;

let notes: MeetingNotes;
beforeEach(() => {
  notes = {
    sections: [],
    blocks: [
      block("t1", "tldr", "ai", "The beta ships on the 12th.", [cite]),
      block("n1", "note", "user", "pricing tiers?"),
      block("e1", "enhanced:n1", "ai", "", []),
    ],
    actionItems: [
      {
        gid: "a1",
        text: "Send the invoice",
        ownerSpeakerGid: null,
        dueText: null,
        done: false,
        origin: "ai",
        citations: [],
      },
    ],
  };
  commands.meetingNotes.mockImplementation(() => ok(structuredClone(notes)));
  for (const k of [
    "updateNoteBlock",
    "deleteNoteBlock",
    "setActionDone",
    "setActionOwner",
    "updateActionItem",
    "deleteActionItem",
  ] as const)
    commands[k].mockImplementation(() => ok(null));
  commands.addNoteBlock.mockImplementation((_m: string, text: string) =>
    ok(block("n9", "note", "user", text)),
  );
  commands.addActionItem.mockImplementation((_m: string, text: string) =>
    ok({
      gid: "a9",
      text,
      ownerSpeakerGid: null,
      dueText: null,
      done: false,
      origin: "user",
      citations: [],
    }),
  );
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const open = async () => {
  renderLive(<NotesTab meeting="m1" detail={detail} />);
  return screen
    .findByText("The beta ships on the 12th.", { selector: "textarea" })
    .catch(() => screen.findByDisplayValue("The beta ships on the 12th."));
};

describe("NotesTab", () => {
  it("shows the app's text as written by the app, and yours as yours", async () => {
    await open();
    expect(
      screen.getAllByText("Written by Ghira from the transcript").length,
    ).toBeGreaterThan(0);
    // The legend, and the one block of yours (its own label is for screen readers).
    expect(screen.getAllByText("You wrote")).toHaveLength(2);
    expect(
      screen.getByText("Not found in the transcript. Kept as your note."),
    ).toBeTruthy();
  });

  it("an AI block you edit becomes yours at once and is saved", async () => {
    const user = userEvent.setup();
    const ta = await open();
    expect(screen.queryByText("Edited by you · kept on regenerate")).toBeNull();
    await user.type(ta, " Maybe.");
    await user.tab();
    await waitFor(() =>
      expect(commands.updateNoteBlock).toHaveBeenCalledWith(
        "m1",
        "t1",
        "The beta ships on the 12th. Maybe.",
      ),
    );
    expect(
      await screen.findByText("Edited by you · kept on regenerate"),
    ).toBeTruthy();
  });

  it("rolls the edit back and says so when the core refuses", async () => {
    commands.updateNoteBlock.mockImplementation(() =>
      Promise.resolve({ status: "error", error: "disk full" }),
    );
    const user = userEvent.setup();
    const ta = await open();
    await user.type(ta, "!");
    await user.tab();
    expect(await screen.findByText(/disk full/)).toBeTruthy();
    await waitFor(() =>
      expect(
        screen.queryByText("Edited by you · kept on regenerate"),
      ).toBeNull(),
    );
  });

  it("My notes only hides what the app wrote and keeps edited text", async () => {
    renderLive(<NotesTab meeting="m1" detail={detail} onlyMine />);
    await screen.findByDisplayValue("pricing tiers?");
    expect(
      screen.queryByDisplayValue("The beta ships on the 12th."),
    ).toBeNull();
    expect(screen.queryByText("Send the invoice")).toBeNull();
    cleanup();
    renderLive(<NotesTab meeting="m1" detail={detail} />);
    expect(
      await screen.findByDisplayValue("The beta ships on the 12th."),
    ).toBeTruthy();
  });

  it("Enter in the note row adds a user note and keeps the row for the next one", async () => {
    const user = userEvent.setup();
    await open();
    const row = screen.getByLabelText("Add a note") as HTMLTextAreaElement;
    await user.type(row, "ask legal{Enter}");
    await waitFor(() =>
      expect(commands.addNoteBlock).toHaveBeenCalledWith("m1", "ask legal"),
    );
    expect(row.value).toBe("");
    expect(await screen.findByDisplayValue("ask legal")).toBeTruthy();
  });

  it("does not add a note for the Enter that confirms an IME composition", async () => {
    await open();
    const row = screen.getByLabelText("Add a note");
    fireEvent.change(row, { target: { value: "ghi chú" } });
    fireEvent.keyDown(row, { key: "Enter", isComposing: true });
    fireEvent.keyDown(row, { key: "Enter", keyCode: 229 });
    fireEvent.compositionStart(row);
    fireEvent.keyDown(row, { key: "Enter" });
    expect(commands.addNoteBlock).not.toHaveBeenCalled();
    fireEvent.compositionEnd(row);
    fireEvent.keyDown(row, { key: "Enter" });
    await waitFor(() =>
      expect(commands.addNoteBlock).toHaveBeenCalledWith("m1", "ghi chú"),
    );
  });

  it("Backspace in an empty line of yours deletes it", async () => {
    const user = userEvent.setup();
    await open();
    const line = screen.getByDisplayValue("pricing tiers?");
    await user.clear(line);
    await user.keyboard("{Backspace}");
    await waitFor(() =>
      expect(commands.deleteNoteBlock).toHaveBeenCalledWith("m1", "n1"),
    );
    await waitFor(() =>
      expect(screen.queryByDisplayValue("pricing tiers?")).toBeNull(),
    );
  });

  it("ticks, adds and deletes action items", async () => {
    const user = userEvent.setup();
    await open();
    await user.click(screen.getByRole("checkbox", { name: "Done" }));
    expect(commands.setActionDone).toHaveBeenCalledWith("m1", "a1", true);
    await user.type(
      screen.getByLabelText("Add an action item"),
      "book room{Enter}",
    );
    await waitFor(() =>
      expect(commands.addActionItem).toHaveBeenCalledWith(
        "m1",
        "book room",
        null,
      ),
    );
    await user.click(screen.getAllByRole("button", { name: "Delete" })[0]!);
    expect(commands.deleteActionItem).toHaveBeenCalledWith("m1", "a1");
  });

  it("picks an owner from the speakers", async () => {
    const user = userEvent.setup();
    await open();
    await user.click(screen.getByRole("button", { name: /Owner: Unassigned/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Sarah" }));
    expect(commands.setActionOwner).toHaveBeenCalledWith("m1", "a1", "s1");
  });
});

describe("CitationLink", () => {
  it("plays the cited span on click", async () => {
    const play = vi
      .spyOn(usePlayer.getState(), "playSpan")
      .mockImplementation(() => undefined);
    const user = userEvent.setup();
    renderLive(
      <CitationLink
        citation={cite}
        speakers={detail.speakers}
        audioAvailable
      />,
    );
    await user.click(
      screen.getByRole("button", { name: "Show in transcript 00:12" }),
    );
    expect(play).toHaveBeenCalledWith(12_000, 15_000);
  });

  it("is dashed and not playable without audio", async () => {
    const play = vi
      .spyOn(usePlayer.getState(), "playSpan")
      .mockImplementation(() => undefined);
    const user = userEvent.setup();
    renderLive(
      <CitationLink
        citation={cite}
        speakers={detail.speakers}
        audioAvailable={false}
      />,
    );
    const chip = screen.getByRole("button", { name: /The audio was deleted/ });
    expect(chip.getAttribute("data-state")).toBe("broken");
    await user.click(chip);
    expect(play).not.toHaveBeenCalled();
  });

  it("previews the quote and speaker on focus", async () => {
    renderLive(
      <CitationLink
        citation={cite}
        speakers={detail.speakers}
        audioAvailable
      />,
    );
    fireEvent.focus(
      screen.getByRole("button", { name: "Show in transcript 00:12" }),
    );
    const preview = await screen.findByRole("group", { name: "Quote preview" });
    expect(preview.textContent).toContain("we ship on the 12th");
    expect(preview.textContent).toContain("Sarah");
    expect(screen.getByRole("button", { name: "Play from 00:12" })).toBeTruthy();
  });
});

describe("typing pauses (debounce)", () => {
  beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));
  afterEach(() => vi.useRealTimers());
  const typer = () =>
    userEvent.setup({ advanceTimers: vi.advanceTimersByTime });

  it("the add-a-note row waits for Enter or blur: a pause does not save half a note", async () => {
    const user = typer();
    await open();
    const row = screen.getByLabelText("Add a note") as HTMLTextAreaElement;
    await user.type(row, "follow");
    await vi.advanceTimersByTimeAsync(2000);
    expect(commands.addNoteBlock).not.toHaveBeenCalled();
    await user.type(row, " up{Enter}");
    await waitFor(() => expect(commands.addNoteBlock).toHaveBeenCalledTimes(1));
    expect(commands.addNoteBlock).toHaveBeenCalledWith("m1", "follow up");
    expect(row.value).toBe("");
  });

  it("the add row saves once on blur and clears, so nothing is added twice", async () => {
    const user = typer();
    await open();
    const row = screen.getByLabelText("Add a note") as HTMLTextAreaElement;
    await user.type(row, "call Linh");
    await user.tab();
    await vi.advanceTimersByTimeAsync(2000);
    expect(commands.addNoteBlock).toHaveBeenCalledTimes(1);
    expect(row.value).toBe("");
  });

  it("the add-action row behaves the same", async () => {
    const user = typer();
    await open();
    const row = screen.getByLabelText("Add an action item");
    await user.type(row, "book");
    await vi.advanceTimersByTimeAsync(2000);
    expect(commands.addActionItem).not.toHaveBeenCalled();
    await user.type(row, " room{Enter}");
    await waitFor(() =>
      expect(commands.addActionItem).toHaveBeenCalledWith(
        "m1",
        "book room",
        null,
      ),
    );
  });

  it("keeps a trailing space typed before the save lands", async () => {
    const user = typer();
    const ta = (await open()) as HTMLTextAreaElement;
    await user.type(ta, " Maybe ");
    await vi.advanceTimersByTimeAsync(1000);
    await waitFor(() => expect(commands.updateNoteBlock).toHaveBeenCalled());
    await user.type(ta, "so");
    expect(ta.value).toBe("The beta ships on the 12th. Maybe so");
  });

  it("emptying a note to retype it does not delete it", async () => {
    const user = typer();
    await open();
    const line = screen.getByDisplayValue("pricing tiers?");
    await user.clear(line);
    await vi.advanceTimersByTimeAsync(2000);
    expect(commands.deleteNoteBlock).not.toHaveBeenCalled();
    await user.type(line, "new wording");
    await user.tab();
    await waitFor(() =>
      expect(commands.updateNoteBlock).toHaveBeenCalledWith(
        "m1",
        "n1",
        "new wording",
      ),
    );
    expect(commands.deleteNoteBlock).not.toHaveBeenCalled();
  });

  it("emptying an AI block and leaving restores its text", async () => {
    const user = typer();
    const ta = (await open()) as HTMLTextAreaElement;
    await user.clear(ta);
    await user.tab();
    expect(ta.value).toBe("The beta ships on the 12th.");
    expect(commands.updateNoteBlock).not.toHaveBeenCalled();
  });
});
