// SPDX-License-Identifier: Apache-2.0
import { act, cleanup, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LineInfo, SpeakerInfo } from "../../bindings";
import { initialLive, useLive } from "../../state/live";
import { LiveBanners, MANY_VOICES_MS } from "./banners";
import { SpeakerStrip } from "./speaker-strip";
import { TranscriptView } from "./transcript-view";
import { renderLive, setLive } from "./test-utils";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  useLive.setState(initialLive);
});

const sp = (id: number): SpeakerInfo => ({ id, label: `Speaker ${id}`, colorSlot: id <= 8 ? id : 0, isMe: false, provisional: false, notPerson: false, others: id > 8 });
const speakers = (n: number) => Object.fromEntries(Array.from({ length: n }, (_, i) => [i + 1, sp(i + 1)]));
const line = (speaker: number, t0Ms: number, t1Ms: number, text: string, overlap = false): LineInfo => ({ gid: `l${t0Ms}`, speaker, t0Ms, t1Ms, text, overlap, words: [] });

describe("more than eight voices", () => {
  it("shows Others · N on the lane and a +N chip listing who is inside", () => {
    setLive({ state: "recording", meeting: "m", speakers: speakers(11) });
    renderLive(<SpeakerStrip />);
    const chip = screen.getByTestId("others-chip");
    expect(chip.textContent).toBe("+3Others");
    expect(chip.getAttribute("aria-label")).toBe("3 voices in Others");
    // The timeline is closed until asked for.
    expect(screen.queryByText("Others · 3")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Timeline/ }));
    expect(screen.getAllByText("Others · 3").length).toBeGreaterThan(0);
    // Eight chips of their own: the nine to eleven are not listed one by one.
    expect(within(screen.getByRole("list", { name: "Speakers" })).queryByText("Speaker 9")).toBeNull();
    fireEvent.click(chip);
    expect(within(screen.getByRole("dialog")).getByText("Speaker 11")).toBeTruthy();
  });

  it("has no Others chip up to eight", () => {
    setLive({ state: "recording", meeting: "m", speakers: speakers(8) });
    renderLive(<SpeakerStrip />);
    expect(screen.queryByTestId("others-chip")).toBeNull();
  });

  it("tells once, when the first voice lands in Others, and then goes away", () => {
    vi.useFakeTimers();
    setLive({ state: "recording", meeting: "m", speakers: speakers(8) });
    renderLive(<LiveBanners />);
    expect(document.querySelector("[data-banner=many-voices]")).toBeNull();
    act(() => useLive.setState({ speakers: speakers(9) }));
    expect(document.querySelectorAll("[data-banner=many-voices]")).toHaveLength(1);
    expect(screen.getByText(/More than 8 voices/)).toBeTruthy();
    // More arrivals don't make a second one.
    act(() => useLive.setState({ speakers: speakers(11) }));
    expect(document.querySelectorAll("[data-banner=many-voices]")).toHaveLength(1);
    act(() => void vi.advanceTimersByTime(MANY_VOICES_MS + 10));
    expect(document.querySelector("[data-banner=many-voices]")).toBeNull();
    act(() => useLive.setState({ speakers: speakers(12) }));
    expect(document.querySelector("[data-banner=many-voices]")).toBeNull();
  });
});

describe("overlap in the live transcript", () => {
  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(80);
    vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(800);
  });
  const lines = [line(1, 0, 6000, "Chúng ta cần chốt", true), line(2, 4000, 9000, "Cho tôi nói với", true), line(1, 20_000, 22_000, "Tiếp theo nhé")];

  it("flagged overlapping lines stack in one bracket, the hint said once", () => {
    setLive({ state: "recording", meeting: "m", speakers: speakers(2), lines, session: { mode: "room", language: null, title: "", consentConfirmed: false, sensitive: false } });
    renderLive(<TranscriptView />);
    const stack = screen.getByTestId("transcript-stack");
    expect(within(stack).getByText(/Chúng/)).toBeTruthy();
    expect(within(stack).getByText(/Cho/)).toBeTruthy();
    // The header and the two lines each carry the short label; only the header the hint.
    const tags = within(stack).getAllByTestId("overlap-tag");
    expect(tags).toHaveLength(3);
    expect(tags.filter((t) => t.getAttribute("title") !== null)).toHaveLength(1);
    expect(within(stack).queryByText(/Tiếp/)).toBeNull();
  });

  it("lines that overlap in time but are not flagged stay plain lines, in any mode", () => {
    const plain = lines.map((l) => ({ ...l, overlap: false }));
    for (const mode of ["call", "room"]) {
      setLive({ state: "recording", meeting: "m", speakers: speakers(2), lines: plain, session: { mode, language: null, title: "", consentConfirmed: false, sensitive: false } });
      const { unmount } = renderLive(<TranscriptView />);
      expect(screen.queryByTestId("transcript-stack")).toBeNull();
      expect(screen.queryAllByTestId("overlap-tag")).toHaveLength(0);
      unmount();
    }
  });
});
