// SPDX-License-Identifier: Apache-2.0
import userEvent from "@testing-library/user-event";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AskAllAnswer } from "../../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const day = 86_400_000;
const commands = vi.hoisted(() => ({
  askAllMeetings: vi.fn(),
  listMeetings: vi.fn(),
  modelsStatus: vi.fn(),
  listPeople: vi.fn(),
}));
const navigate = vi.hoisted(() => vi.fn());
vi.mock("../../ipc", () => ({ ipc: { commands, onCoreEvent: () => Promise.resolve(() => {}) } }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));

import { renderLive } from "../live/test-utils";
import { AskScreenBody } from "./ask-screen";

const row = (gid: string, title: string, ago: number, people: string[] = []) => ({ gid, title, startedAt: Date.now() - ago * day, status: "ready", job: null, people: people.map((name) => ({ name, colorSlot: 2 })) });
const ref = (gid: string, title: string) => ({ meeting: gid, title, startedAt: null });
const answered: AskAllAnswer = {
  answered: true,
  text: "Beta ships in November.",
  citations: [
    { meeting: ref("m1", "Product sync"), citation: { t0Ms: 65_000, t1Ms: 70_000, quote: "q", speakerGid: null, stale: false, missing: false } },
    { meeting: ref("m2", "Standup"), citation: { t0Ms: 5_000, t1Ms: 9_000, quote: "q", speakerGid: null, stale: false, missing: false } },
  ],
  searched: [],
  sources: [ref("m1", "Product sync"), ref("m2", "Standup")],
  semantic: true,
};
const ask = (q: string) => {
  const input = screen.getByRole("textbox");
  fireEvent.change(input, { target: { value: q } });
  fireEvent.keyDown(input, { key: "Enter" });
};

beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset());
  navigate.mockReset();
  commands.listMeetings.mockImplementation((_n: number, offset: number) => ok(offset ? [] : [row("m1", "Product sync", 2, ["Linh"]), row("m2", "Standup", 20, ["Linh"]), row("m3", "Old", 200)]));
  commands.listPeople.mockReturnValue(
    ok({
      thirdParty: false,
      people: [
        { gid: "me", name: "", isMe: true, colorSlot: 1, meetings: 3, openActions: 0, lastMetMs: null, voice: { kind: "self", atMs: null } },
        { gid: "linh", name: "Linh", isMe: false, colorSlot: 2, meetings: 2, openActions: 0, lastMetMs: null, voice: { kind: "none", atMs: null } },
        { gid: "ghost", name: "Ghost", isMe: false, colorSlot: 3, meetings: 0, openActions: 0, lastMetMs: null, voice: { kind: "none", atMs: null } },
      ],
    }),
  );
  commands.modelsStatus.mockReturnValue(ok({ tier: "balanced", models: [{ id: "qwen3-4b", role: "llm", installed: true }], downloading: false }));
});
afterEach(cleanup);

describe("Ask across meetings", () => {
  it("answers with citation chips that open the meeting at the moment, and a footer", async () => {
    commands.askAllMeetings.mockReturnValue(ok(answered));
    renderLive(<AskScreenBody />);
    await screen.findByText(/Searching 3 meetings/);
    ask("When does the beta ship?");
    expect(commands.askAllMeetings).toHaveBeenCalledWith("When does the beta ship?", { meetings: [], fromMs: null, toMs: null, persons: [] }, "en");
    expect(await screen.findByText("Beta ships in November.")).toBeTruthy();
    expect(screen.getByText(/qwen3-4b · 2 meetings read/)).toBeTruthy();
    expect(screen.queryByTestId("ask-keyword-only")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open Product sync at 1:05" }));
    expect(navigate).toHaveBeenCalledWith({ to: "/meetings/$id/$tab", params: { id: "m1", tab: "transcript" }, search: { t: 65_000 } });
  });

  it("shows Thinking while waiting, then clears the thread with New question", async () => {
    let resolve!: (v: unknown) => void;
    commands.askAllMeetings.mockReturnValue(new Promise((r) => (resolve = r)));
    renderLive(<AskScreenBody />);
    await screen.findByText(/Searching 3 meetings/);
    ask("Q?");
    expect((await screen.findAllByText(/Thinking/)).length).toBeGreaterThan(0);
    // New question waits for the answer; the counter is hidden from the live region.
    expect((screen.getByRole("button", { name: "New question" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("Thinking…").className).toContain("sr-only");
    expect(screen.queryByRole("status")).toBeNull();
    await act(async () => resolve({ status: "ok", data: answered }));
    await screen.findByText("Beta ships in November.");
    fireEvent.click(screen.getByRole("button", { name: "New question" }));
    expect(screen.queryByTestId("ask-entry")).toBeNull();
  });

  it("not discussed: searched terms and a button to search transcripts", async () => {
    commands.askAllMeetings.mockReturnValue(ok({ ...answered, answered: false, text: "", citations: [], searched: ["pricing"] }));
    renderLive(<AskScreenBody />);
    await screen.findByText(/Searching 3 meetings/);
    ask("pricing tiers?");
    const card = await screen.findByTestId("ask-not-discussed");
    expect(card.textContent).toContain("pricing");
    fireEvent.click(within(card).getByRole("button"));
    expect(navigate).toHaveBeenCalledWith({ to: "/meetings", search: { q: "pricing" } });
    // Nothing was read, so no "answered on this Mac" footer.
    expect(screen.queryByText(/Answered on this/)).toBeNull();
  });

  it("a refusal code is a neutral note, other errors an alert", async () => {
    commands.askAllMeetings.mockReturnValue(Promise.resolve({ status: "error", error: "busyRecording" }));
    renderLive(<AskScreenBody />);
    await screen.findByText(/Searching 3 meetings/);
    ask("Q?");
    expect((await screen.findByTestId("ask-busy")).textContent).toContain("waits until the recording stops");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("does not send while an IME composition is confirmed with Enter", async () => {
    renderLive(<AskScreenBody />);
    await screen.findByText(/Searching 3 meetings/);
    const input = screen.getByRole("textbox");
    fireEvent.change(input, { target: { value: "nhận" } });
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(commands.askAllMeetings).not.toHaveBeenCalled();
  });

  it("a missing citation opens the notes tab, says why, and has no dangling time", async () => {
    commands.askAllMeetings.mockReturnValue(
      ok({
        ...answered,
        citations: [
          { meeting: ref("m1", "Product sync"), citation: { t0Ms: 65_000, t1Ms: null, quote: "", speakerGid: null, stale: false, missing: true } },
          { meeting: ref("m2", "Standup"), citation: { t0Ms: null, t1Ms: null, quote: "", speakerGid: null, stale: false, missing: false } },
        ],
      }),
    );
    renderLive(<AskScreenBody />);
    await screen.findByText(/Searching 3 meetings/);
    ask("Q?");
    await screen.findByText("Beta ships in November.");
    const [missing, noTime] = screen.getAllByTestId("ask-chip");
    expect(missing!.getAttribute("aria-label")).toMatch(/^Open Product sync at 1:05 · /);
    expect(missing!.title).not.toBe("");
    expect(noTime!.getAttribute("aria-label")).toBe("Open Standup");
    fireEvent.click(missing!);
    expect(navigate).toHaveBeenCalledWith({ to: "/meetings/$id/$tab", params: { id: "m1", tab: "notes" }, search: {} });
  });

  it("the footer names the scope that answered", async () => {
    commands.askAllMeetings.mockReturnValue(ok(answered));
    renderLive(<AskScreenBody />);
    await screen.findByText(/Searching 3 meetings/);
    ask("A?");
    expect(await screen.findByText(/meetings read · All meetings/)).toBeTruthy();
  });

  it("an error from the core is shown as an alert", async () => {
    commands.askAllMeetings.mockReturnValue(Promise.resolve({ status: "error", error: "a recording is in progress" }));
    renderLive(<AskScreenBody />);
    await screen.findByText(/Searching 3 meetings/);
    ask("Q?");
    expect((await screen.findByRole("alert")).textContent).toContain("a recording is in progress");
  });

  it("says when only keywords were searched", async () => {
    commands.askAllMeetings.mockReturnValue(ok({ ...answered, semantic: false }));
    renderLive(<AskScreenBody />);
    await screen.findByText(/Searching 3 meetings/);
    ask("Q?");
    expect(await screen.findByTestId("ask-keyword-only")).toBeTruthy();
  });

  it("scope: this meeting and a date range are sent as the scope", async () => {
    commands.askAllMeetings.mockReturnValue(ok(answered));
    renderLive(<AskScreenBody meeting="m2" />);
    await screen.findByText("Searching “Standup”");
    ask("A?");
    await screen.findByText("Beta ships in November.");
    expect(commands.askAllMeetings.mock.calls[0]![1]).toEqual({ meetings: ["m2"], fromMs: null, toMs: null, persons: [] });
    fireEvent.click(screen.getByRole("radio", { name: "Date range" }));
    await screen.findByText(/Searching Last 30 days · 2 meetings/);
    ask("B?");
    await waitFor(() => expect(commands.askAllMeetings).toHaveBeenCalledTimes(2));
    const scope = commands.askAllMeetings.mock.calls[1]![1];
    expect(scope.meetings).toEqual([]);
    expect(scope.fromMs).toBeLessThan(Date.now() - 28 * day);
    expect(scope.fromMs).toBeGreaterThan(Date.now() - 31 * day);
  });

  it("a suggested question asks it; an empty library shows the empty state", async () => {
    commands.askAllMeetings.mockReturnValue(ok(answered));
    renderLive(<AskScreenBody />);
    fireEvent.click(await screen.findByRole("button", { name: "What did we decide recently?" }));
    expect(commands.askAllMeetings.mock.calls[0]![0]).toBe("What did we decide recently?");
    expect(document.activeElement).toBe(screen.getByRole("textbox"));
    cleanup();
    commands.listMeetings.mockImplementation(() => ok([]));
    renderLive(<AskScreenBody />);
    expect(await screen.findByText("Ask works once you have a few meetings")).toBeTruthy();
  });

  it("A person: the picker offers people from meetings and sends their gid in the scope", async () => {
    commands.askAllMeetings.mockReturnValue(ok(answered));
    renderLive(<AskScreenBody />);
    await screen.findByText(/Searching 3 meetings/);
    fireEvent.click(await screen.findByRole("radio", { name: "A person" }));
    // Nobody is picked yet: asking waits for a choice.
    expect((screen.getByRole("button", { name: "Ask" }) as HTMLButtonElement).disabled).toBe(true);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Choose a person" }));
    expect(screen.queryByRole("menuitem", { name: "Ghost" })).toBeNull();
    await user.click(await screen.findByRole("menuitem", { name: "Linh" }));
    // The count is of meetings the core reads that have her in them.
    await screen.findByText("Searching 2 meetings with Linh");
    ask("What did Linh say?");
    await screen.findByText("Beta ships in November.");
    expect(commands.askAllMeetings.mock.calls[0]![1]).toEqual({ meetings: [], fromMs: null, toMs: null, persons: ["linh"] });
    expect(screen.getByText(/meetings read · Linh/)).toBeTruthy();
  });
});
