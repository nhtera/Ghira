// SPDX-License-Identifier: Apache-2.0
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
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
  setDecisionStatus: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands } }));
const navigate = vi.hoisted(() => vi.fn());
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));

import { renderLive } from "../live/test-utils";
import { usePlayer } from "../../state/player";
import { CitationGroup, CitationLink } from "../citation/citation-link";
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
    marks: [],
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
    "setDecisionStatus",
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
  it("lists saved Ask answers in their own section, pinned, editable and deletable", async () => {
    notes.blocks.push({ ...block("ans1", "answer", "ai", "Q: When?\nA: On the 12th.", [cite]), pinned: true });
    await open();
    const list = within(screen.getByTestId("saved-answers"));
    expect(screen.getByRole("heading", { name: "Saved from Ask" })).toBeTruthy();
    expect(list.getAllByRole("listitem")).toHaveLength(1);
    expect(list.getByDisplayValue(/Q: When\?/)).toBeTruthy();
    expect(list.getByText("Kept when the notes are rewritten")).toBeTruthy();
    // the citation chip works like any other
    expect(list.getByRole("button", { name: "Show in transcript 00:12" })).toBeTruthy();
    fireEvent.click(list.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(commands.deleteNoteBlock).toHaveBeenCalledWith("m1", "ans1"));
  });

  it("a pinned block shows the pin, an unpinned one does not, and no answers means no section", async () => {
    await open();
    expect(screen.queryByText("Kept when the notes are rewritten")).toBeNull();
    expect(screen.queryByTestId("saved-answers")).toBeNull();
  });

  const row = (gid: string) => document.querySelector<HTMLElement>(`[data-block="${gid}"]`)!;

  it("lists decided items, then the proposed ones with a chip and the commitment footnote", async () => {
    notes.blocks.push(block("p1", "proposal", "ai", "Maybe a dark theme.", [cite]), block("d1", "decision", "ai", "Ship on Friday.", [cite]));
    await open();
    const section = within(screen.getByRole("region", { name: "Decisions" }));
    // decided first, whatever the order they came in
    const values = section.getAllByRole("textbox").map((x) => (x as HTMLTextAreaElement).value);
    expect(values).toEqual(["Ship on Friday.", "Maybe a dark theme."]);
    expect(within(row("p1")).getByTestId("proposed-chip").textContent).toBe("Proposed");
    expect(within(row("d1")).queryByTestId("proposed-chip")).toBeNull();
    expect(screen.getAllByTestId("proposed-chip")).toHaveLength(1);
    expect(section.getByTestId("proposal-footnote").textContent).toBe("AI suggestions are not commitments.");
  });

  it("only proposals: the Decisions section still shows, with the footnote", async () => {
    notes.blocks.push(block("p1", "proposal", "ai", "Maybe a dark theme.", [cite]));
    await open();
    expect(screen.getByRole("region", { name: "Decisions" })).toBeTruthy();
    expect(screen.getByTestId("proposal-footnote")).toBeTruthy();
  });

  it("no footnote without proposals", async () => {
    notes.blocks.push(block("d1", "decision", "ai", "Ship on Friday.", [cite]));
    await open();
    expect(screen.queryByTestId("proposal-footnote")).toBeNull();
  });

  it("the block menu moves a decision to Proposed and back, and keyboard focus stays on its menu", async () => {
    const user = userEvent.setup();
    commands.setDecisionStatus.mockImplementation(() => ok(null));
    notes.blocks.push(block("d1", "decision", "ai", "Ship on Friday.", [cite]), block("p1", "proposal", "ai", "Maybe a dark theme.", [cite]));
    await open();
    const menu = () => within(row("d1")).getByRole("button", { name: "Decision options" });
    const before = row("d1");
    await user.click(menu());
    await user.click(await screen.findByRole("menuitem", { name: "Mark as proposed" }));
    expect(commands.setDecisionStatus).toHaveBeenCalledWith("m1", "d1", true);
    await waitFor(() => expect(within(row("d1")).getByTestId("proposed-chip")).toBeTruthy());
    // it is the same row (one keyed list), now listed with the proposals
    expect(row("d1")).toBe(before);
    expect(within(screen.getByRole("region", { name: "Decisions" })).getAllByRole("textbox").map((x) => (x as HTMLTextAreaElement).value)).toEqual(["Ship on Friday.", "Maybe a dark theme."]);
    await waitFor(() => expect(document.activeElement).toBe(menu()));
    // and back
    await user.click(menu());
    await user.click(await screen.findByRole("menuitem", { name: "Mark as decided" }));
    expect(commands.setDecisionStatus).toHaveBeenLastCalledWith("m1", "d1", false);
    await waitFor(() => expect(within(row("d1")).queryByTestId("proposed-chip")).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(menu()));
  });

  it("a refused move shows at once, then puts the decision back and says why; only the app's decisions have a menu", async () => {
    const user = userEvent.setup();
    let refuse!: (v: unknown) => void;
    commands.setDecisionStatus.mockReturnValue(new Promise((r) => (refuse = r)));
    notes.blocks.push(block("d1", "decision", "ai", "Ship on Friday.", [cite]), block("d2", "decision", "user", "My own decision"));
    await open();
    // one menu: the user's own tagged decision has none
    expect(screen.getAllByRole("button", { name: "Decision options" })).toHaveLength(1);
    await user.click(screen.getByRole("button", { name: "Decision options" }));
    await user.click(await screen.findByRole("menuitem", { name: "Mark as proposed" }));
    // optimistic: the chip is there while the core has not answered
    expect(within(row("d1")).getByTestId("proposed-chip")).toBeTruthy();
    expect(screen.getByTestId("proposal-footnote")).toBeTruthy();
    await act(async () => refuse({ status: "error", error: "only a decision the app wrote can be moved" }));
    await waitFor(() => expect(within(row("d1")).queryByTestId("proposed-chip")).toBeNull());
    expect(screen.queryByTestId("proposal-footnote")).toBeNull();
    expect((await screen.findAllByText(/only a decision the app wrote can be moved/)).length).toBeGreaterThan(0);
  });

  it("stars the items that cover a marked moment and lists only the uncovered marks", async () => {
    notes.marks = [
      { tMs: 724_000, tag: "decision", segment: "g1", text: "we ship on the 12th", coveredBy: ["t1", "a1"] },
      { tMs: 1_800_000, tag: "question", segment: "g9", text: "what about the budget", coveredBy: [] },
      { tMs: 2_000_000, tag: "star", segment: null, text: null, coveredBy: [] },
    ];
    await open();
    const stars = screen.getAllByTestId("mark-star");
    expect(stars).toHaveLength(2);
    expect(stars[0]!.getAttribute("title")).toBe("You marked this at 12:04");
    const section = within(screen.getByTestId("marked-moments"));
    expect(section.getAllByRole("listitem")).toHaveLength(2);
    expect(section.getByText("what about the budget")).toBeTruthy();
    expect(section.queryByText("we ship on the 12th")).toBeNull();
    expect(screen.getByRole("heading", { name: "Moments you marked" })).toBeTruthy();
    // play from a mark
    const seek = vi.spyOn(usePlayer.getState(), "seek").mockImplementation(() => undefined);
    fireEvent.click(section.getByRole("button", { name: "Play from 30:00" }));
    expect(seek).toHaveBeenCalledWith(1_800_000, true);
  });

  it("has no star and no Moments section without marks, or when every mark is covered", async () => {
    await open();
    expect(screen.queryByTestId("mark-star")).toBeNull();
    expect(screen.queryByTestId("marked-moments")).toBeNull();
    cleanup();
    notes.marks = [{ tMs: 1000, tag: "star", segment: "g1", text: "x", coveredBy: ["t1"] }];
    await open();
    expect(screen.getAllByTestId("mark-star")).toHaveLength(1);
    expect(screen.queryByTestId("marked-moments")).toBeNull();
  });

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

describe("CitationGroup", () => {
  const three = [cite, { ...cite, t0Ms: 40_000, t1Ms: 44_000, quote: "second source" }, { ...cite, t0Ms: 90_000, t1Ms: 93_000, quote: "third source" }];
  const group = () =>
    renderLive(<CitationGroup citations={three} speakers={detail.speakers} audioAvailable meeting="m1" />);

  it("renders the first chip and a +2 chip, not three chips", () => {
    group();
    expect(screen.getAllByRole("button", { name: /^Show in transcript \d\d:\d\d$/ })).toHaveLength(1);
    expect(screen.getByRole("button", { name: "2 more sources" }).textContent).toBe("+2");
  });

  it("steps through all sources with the buttons and wraps", async () => {
    group();
    fireEvent.click(screen.getByRole("button", { name: "2 more sources" }));
    const preview = await screen.findByRole("group", { name: "Quote preview" }, { timeout: 3000 });
    expect(preview.textContent).toContain("second source");
    expect(preview.textContent).toContain("2/3");
    fireEvent.click(screen.getByRole("button", { name: "Next source" }));
    expect(preview.textContent).toContain("third source");
    fireEvent.click(screen.getByRole("button", { name: "Next source" }));
    expect(preview.textContent).toContain("we ship on the 12th");
    expect(preview.textContent).toContain("1/3");
    fireEvent.click(screen.getByRole("button", { name: "Previous source" }));
    expect(preview.textContent).toContain("3/3");
  });

  it("steps with the arrow keys and plays the shown source", async () => {
    const play = vi.spyOn(usePlayer.getState(), "playSpan").mockImplementation(() => undefined);
    group();
    fireEvent.click(screen.getByRole("button", { name: "2 more sources" }));
    const preview = await screen.findByRole("group", { name: "Quote preview" }, { timeout: 3000 });
    fireEvent.keyDown(preview, { key: "ArrowRight" });
    expect(preview.textContent).toContain("3/3");
    fireEvent.keyDown(preview, { key: "ArrowLeft" });
    fireEvent.keyDown(preview, { key: "ArrowLeft" });
    expect(preview.textContent).toContain("1/3");
    fireEvent.keyDown(preview, { key: "ArrowRight" });
    fireEvent.click(screen.getByRole("button", { name: "Play from 00:40" }));
    expect(play).toHaveBeenCalledWith(40_000, 44_000);
  });

  it("Show in transcript opens the transcript tab at the shown source", async () => {
    group();
    fireEvent.click(screen.getByRole("button", { name: "2 more sources" }));
    await screen.findByRole("group", { name: "Quote preview" }, { timeout: 3000 });
    fireEvent.click(screen.getByRole("button", { name: "Next source" }));
    fireEvent.click(screen.getByRole("button", { name: "Show in transcript" }));
    expect(navigate).toHaveBeenCalledWith({ to: "/meetings/$id/$tab", params: { id: "m1", tab: "transcript" }, search: { t: 90_000 } });
  });

  it("names the position for assistive tech and starts at the first source after closing", async () => {
    group();
    fireEvent.click(screen.getByRole("button", { name: "2 more sources" }));
    const preview = await screen.findByRole("group", { name: "Quote preview" }, { timeout: 3000 });
    expect(within(preview).getByRole("status").textContent).toContain("Source 2 of 3");
    expect(preview.querySelector("[aria-live]")).toBeNull();
    fireEvent.keyDown(preview, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("group", { name: "Quote preview" })).toBeNull());
    fireEvent.focus(screen.getByRole("button", { name: /^Show in transcript \d\d:\d\d$/ }));
    const again = await screen.findByRole("group", { name: "Quote preview" }, { timeout: 3000 });
    expect(again.textContent).toContain("1/3");
  });

  it("a single source has no +n and no stepper", async () => {
    renderLive(<CitationGroup citations={[cite]} speakers={detail.speakers} audioAvailable meeting="m1" />);
    expect(screen.queryByRole("button", { name: /more source/ })).toBeNull();
    fireEvent.focus(screen.getByRole("button", { name: "Show in transcript 00:12" }));
    await screen.findByRole("group", { name: "Quote preview" });
    expect(screen.queryByRole("button", { name: "Next source" })).toBeNull();
    expect(screen.getByRole("button", { name: "Show in transcript" })).toBeTruthy();
  });
});
