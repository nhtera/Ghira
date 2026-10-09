// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TalkShare } from "./talk-share";

afterEach(cleanup);

const labels = new Map([
  ["a", { name: "Linh", colorSlot: 1, isMe: false }],
  ["m", { name: "Video", colorSlot: 2, isMe: false }],
]);

describe("TalkShare", () => {
  it("counts only the given people as speakers", () => {
    const entries = [
      { gid: "a", talkMs: 6000, pct: 60, turns: 2 },
      { gid: "m", talkMs: 4000, pct: 40, turns: 1 },
    ];
    render(<TalkShare entries={entries} labels={labels} speakerCount={1} onOpenSpeaker={vi.fn()} />);
    expect(screen.getByTestId("talk-share").textContent).toContain("1 speaker · 3 turns");
  });

  it("is left out without any speaker, never '0 speakers'", () => {
    render(<TalkShare entries={[{ gid: null, talkMs: 5000, pct: 100, turns: 4 }]} labels={labels} speakerCount={0} onOpenSpeaker={vi.fn()} />);
    expect(screen.queryByTestId("talk-share")).toBeNull();
  });
});
