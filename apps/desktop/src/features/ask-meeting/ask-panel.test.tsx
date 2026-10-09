// SPDX-License-Identifier: Apache-2.0
import {
  act,
  cleanup,
  fireEvent,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AskAnswer, MeetingDetail } from "../../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const commands = vi.hoisted(() => ({
  askMeeting: vi.fn(),
  cloudKeys: vi.fn(),
  cloudModels: vi.fn(),
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
  cloudPreview: vi.fn(),
  cloudSend: vi.fn(),
  meetingNotes: vi.fn(),
  saveAnswer: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands } }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));

import { usePlayer } from "../../state/player";
import { renderLive } from "../live/test-utils";
import { AskPanel } from "./ask-panel";

const detail = {
  gid: "m1",
  audioAvailable: true,
  cloudLocked: false,
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
  ],
} as unknown as MeetingDetail;

const answered: AskAnswer = {
  answered: true,
  text: "We chose NeMo for diarization.",
  citations: [
    {
      t0Ms: 12_000,
      t1Ms: 15_000,
      quote: "nhận diện người nói",
      speakerGid: "s1",
      stale: false,
      missing: false,
    },
  ],
  searched: [],
  engine: "local",
  id: "ans-1",
};

const ask = (q: string) => {
  const input = screen.getByRole("textbox");
  fireEvent.change(input, { target: { value: q } });
  fireEvent.keyDown(input, { key: "Enter" });
};

beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset());
  commands.cloudKeys.mockReturnValue(ok([]));
  commands.cloudModels.mockResolvedValue([]);
  commands.getSettings.mockReturnValue(
    ok({
      cloudProvider: "openai",
      cloudModel: "",
      cloudRedact: true,
      cloudOffered: true,
      strictOffline: false,
    }),
  );
  commands.meetingNotes.mockReturnValue(ok({ blocks: [], actionItems: [], sections: [], marks: [] }));
  renderLive(<AskPanel meeting="m1" detail={detail} onClose={vi.fn()} />);
});
afterEach(() => cleanup());

describe("AskPanel", () => {
  it("Enter asks on this device, shows Thinking with seconds, then the answer and its footer", async () => {
    let resolve!: (v: unknown) => void;
    commands.askMeeting.mockReturnValue(new Promise((r) => (resolve = r)));
    ask("Why NeMo?");
    expect(commands.askMeeting).toHaveBeenCalledWith(
      "m1",
      "Why NeMo?",
      "meeting",
    );
    expect(
      await screen.findByText(/ask\.meeting\.thinking|Thinking/),
    ).toBeTruthy();
    await act(async () => resolve({ status: "ok", data: answered }));
    expect(
      await screen.findByText("We chose NeMo for diarization."),
    ).toBeTruthy();
    expect(
      screen.getByText(/ask\.meeting\.answeredLocal|Answered on this device/),
    ).toBeTruthy();
  });

  it("does not send while an IME composition is confirmed with Enter", () => {
    const input = screen.getByRole("textbox");
    fireEvent.change(input, { target: { value: "nhận" } });
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(commands.askMeeting).not.toHaveBeenCalled();
  });

  it("an unanswered question is a not-discussed card with the searched terms", async () => {
    commands.askMeeting.mockReturnValue(
      ok({
        answered: false,
        text: "",
        citations: [],
        searched: ["kubernetes"],
        engine: "local",
      }),
    );
    ask("kubernetes?");
    const card = await screen.findByTestId("ask-not-discussed");
    expect(card.textContent).toContain("kubernetes");
  });

  it("a citation chip plays the cited span", async () => {
    const playSpan = vi.fn();
    usePlayer.setState({ playSpan } as never);
    commands.askMeeting.mockReturnValue(ok(answered));
    ask("Why NeMo?");
    await screen.findByText("We chose NeMo for diarization.");
    fireEvent.click(screen.getByRole("button", { name: /0:12|00:12/ }));
    expect(playSpan).toHaveBeenCalledWith(12_000, 15_000);
  });

  it("shows errors from the core", async () => {
    commands.askMeeting.mockReturnValue(
      Promise.resolve({ status: "error", error: "busyNotes" }),
    );
    ask("anything");
    expect((await screen.findByTestId("ask-busy")).textContent).toContain(
      "Notes are being written",
    );
    expect(screen.queryByRole("alert")).toBeNull();
    commands.askMeeting.mockReturnValue(
      Promise.resolve({ status: "error", error: "model exploded" }),
    );
    ask("again");
    expect((await screen.findByRole("alert")).textContent).toContain(
      "model exploded",
    );
  });

  it("Cloud mode opens the send sheet instead of asking locally; a failed send falls back to the device", async () => {
    commands.cloudKeys.mockReturnValue(
      ok([{ provider: "openai", stored: true }]),
    );
    commands.cloudModels.mockResolvedValue([
      { provider: "openai", model: "gpt-4.1-mini" },
    ]);
    commands.cloudPreview.mockReturnValue(
      ok({
        kind: "preview",
        id: "p1",
        provider: "openai",
        model: "gpt-4.1-mini",
        host: "api.openai.com",
        payload: "{}",
        sha256: "ab".repeat(32),
        tokensEst: 10,
        costEstUsd: null,
        retentionNote: "",
        warnings: [],
        redactions: [],
      }),
    );
    commands.cloudSend.mockReturnValue(
      ok({ kind: "failed", reason: "503", leftDevice: true }),
    );
    commands.askMeeting.mockReturnValue(ok(answered));
    fireEvent.click(await screen.findByRole("radio", { name: /cloud/i }));
    ask("Why NeMo?");
    expect(commands.askMeeting).not.toHaveBeenCalled();
    const send = await screen.findByRole("button", {
      name: /cloud\.send|Send and improve/,
    });
    await waitFor(() => expect(send.hasAttribute("disabled")).toBe(false), {
      timeout: 2000,
    });
    fireEvent.click(send);
    await waitFor(() => expect(commands.cloudSend).toHaveBeenCalledWith("p1"));
    expect(
      await screen.findByText("We chose NeMo for diarization."),
    ).toBeTruthy();
    expect(screen.getByText(/cloudFailed|503/)).toBeTruthy();
  });

  it("shows three starter questions that send on click", async () => {
    commands.askMeeting.mockReturnValue(ok(answered));
    const starters = screen.getByRole("list", { name: "Try asking" });
    expect(starters.querySelectorAll("button")).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "What was decided?" }));
    expect(commands.askMeeting).toHaveBeenCalledWith("m1", "What was decided?", "meeting");
    await screen.findByText("We chose NeMo for diarization.");
    // Once there is a conversation the starters are gone.
    expect(screen.queryByRole("list", { name: "Try asking" })).toBeNull();
  });

  it("a starter is reachable and sent with Enter", async () => {
    const user = userEvent.setup();
    commands.askMeeting.mockReturnValue(ok(answered));
    const b = screen.getByRole("button", { name: "What are the action items and who owns them?" });
    b.focus();
    await user.keyboard("{Enter}");
    expect(commands.askMeeting).toHaveBeenCalledWith("m1", "What are the action items and who owns them?", "meeting");
  });

  it("answers with several sources are one chip group with +n", async () => {
    commands.askMeeting.mockReturnValue(ok({ ...answered, citations: [answered.citations[0]!, { ...answered.citations[0]!, t0Ms: 40_000, t1Ms: 42_000 }] }));
    ask("Why NeMo?");
    await screen.findByText("We chose NeMo for diarization.");
    expect(screen.getAllByRole("button", { name: /^Show in transcript \d\d:\d\d$/ })).toHaveLength(1);
    expect(screen.getByRole("button", { name: "1 more source" }).textContent).toBe("+1");
  });

  const remount = (blocks: { kind: string; text: string }[]) => {
    cleanup();
    commands.meetingNotes.mockReturnValue(ok({ blocks: blocks.map((b, i) => ({ gid: `b${i}`, origin: "ai", pinned: false, citations: [], ...b })), actionItems: [], sections: [], marks: [] }));
    renderLive(<AskPanel meeting="m1" detail={detail} onClose={vi.fn()} />);
  };

  it("starts with the meeting's open questions, then makes up three with the static ones", async () => {
    commands.askMeeting.mockReturnValue(ok(answered));
    remount([
      { kind: "question", text: "Who owns the budget?" },
      { kind: "question", text: "Khi nào export xong？" },
      { kind: "question", text: "We still need a date" },
      { kind: "decision", text: "Is this a decision?" },
    ]);
    const starters = within(screen.getByRole("list", { name: "Try asking" }));
    await waitFor(() => expect(starters.getByRole("button", { name: "Who owns the budget?" })).toBeTruthy());
    const names = starters.getAllByRole("button").map((b) => b.textContent);
    expect(names).toEqual(["Who owns the budget?", "Khi nào export xong？", "What was decided?"]);
    fireEvent.click(starters.getByRole("button", { name: "Khi nào export xong？" }));
    expect(commands.askMeeting).toHaveBeenCalledWith("m1", "Khi nào export xong？", "meeting");
  });

  it("shows only three open questions, and none when the notes have no real questions", async () => {
    remount(["a", "b", "c", "d"].map((x) => ({ kind: "question", text: `Question ${x}?` })));
    const starters = within(screen.getByRole("list", { name: "Try asking" }));
    await waitFor(() => expect(starters.getByRole("button", { name: "Question a?" })).toBeTruthy());
    expect(starters.getAllByRole("button").map((b) => b.textContent)).toEqual(["Question a?", "Question b?", "Question c?"]);
    remount([{ kind: "question", text: "just a statement" }]);
    expect(screen.getAllByRole("button", { name: /decided|action items|themes/ })).toHaveLength(3);
  });

  it("Save to notes sends the answer's id, never its text, and says so", async () => {
    commands.askMeeting.mockReturnValue(ok(answered));
    commands.saveAnswer.mockReturnValue(ok(null));
    ask("Why NeMo?");
    await screen.findByText("We chose NeMo for diarization.");
    fireEvent.click(screen.getByRole("button", { name: "Save to notes" }));
    await waitFor(() => expect(commands.saveAnswer).toHaveBeenCalledWith("m1", "ans-1"));
    expect(commands.saveAnswer.mock.calls[0]).toHaveLength(2);
    const done = await screen.findByRole("button", { name: "Saved to notes" });
    expect((done as HTMLButtonElement).disabled).toBe(true);
  });

  it("a second click while the save is on its way does not ask twice", async () => {
    commands.askMeeting.mockReturnValue(ok(answered));
    let done!: (v: unknown) => void;
    commands.saveAnswer.mockReturnValue(new Promise((r) => (done = r)));
    ask("Why NeMo?");
    await screen.findByText("We chose NeMo for diarization.");
    const save = screen.getByRole("button", { name: "Save to notes" });
    fireEvent.click(save);
    fireEvent.click(save);
    expect((save as HTMLButtonElement).disabled).toBe(true);
    expect(commands.saveAnswer).toHaveBeenCalledTimes(1);
    await act(async () => done({ status: "ok", data: null }));
    expect(await screen.findByRole("button", { name: "Saved to notes" })).toBeTruthy();
  });

  it("an expired or full save is said in words", async () => {
    commands.askMeeting.mockReturnValue(ok(answered));
    commands.saveAnswer.mockReturnValue(Promise.resolve({ status: "error" as const, error: "answerExpired" }));
    ask("Why NeMo?");
    await screen.findByText("We chose NeMo for diarization.");
    fireEvent.click(screen.getByRole("button", { name: "Save to notes" }));
    expect((await screen.findAllByText("Ask again to save this answer.")).length).toBeGreaterThan(0);
    // still savable (nothing was written)
    expect((screen.getByRole("button", { name: "Save to notes" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("a not-discussed answer has nothing to save", async () => {
    commands.askMeeting.mockReturnValue(ok({ answered: false, text: "", citations: [], searched: ["x"], engine: "local", id: null }));
    ask("kubernetes?");
    await screen.findByTestId("ask-not-discussed");
    expect(screen.queryByRole("button", { name: "Save to notes" })).toBeNull();
  });
});
