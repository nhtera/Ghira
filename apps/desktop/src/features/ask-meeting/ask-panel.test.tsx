// SPDX-License-Identifier: Apache-2.0
import {
  act,
  cleanup,
  fireEvent,
  screen,
  waitFor,
} from "@testing-library/react";
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
});
