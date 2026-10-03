// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TranscriptLine, wordsFromText } from "./transcript-line";

afterEach(cleanup);

const speaker = { label: "Minh", colorSlot: 4 };

describe("wordsFromText", () => {
  it("flags the words listed as low confidence", () => {
    expect(wordsFromText("export sang CoreML.", "CoreML.")).toEqual([
      { text: "export", lowConfidence: false },
      { text: "sang", lowConfidence: false },
      { text: "CoreML.", lowConfidence: true },
    ]);
  });
});

describe("TranscriptLine", () => {
  it("final: time, speaker name and the text as plain text", () => {
    const { container } = render(<TranscriptLine startMs={572_000} speaker={speaker} words={wordsFromText("Rename thì dễ.")} />);
    expect(screen.getByText("9:32")).toBeTruthy();
    expect(screen.getByText("Minh")).toBeTruthy();
    expect(container.querySelector("p")?.textContent).toBe("Rename thì dễ.");
    expect(container.querySelector("p")?.getAttribute("aria-live")).toBe("off");
    expect(container.querySelector("[data-state]")?.getAttribute("data-state")).toBe("final");
  });

  it("never renders markup from the text (RT-6)", () => {
    const { container } = render(<TranscriptLine startMs={0} speaker={speaker} words={wordsFromText("<img src=x onerror=alert(1)> <b>hi</b>")} />);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("p b")).toBeNull();
    expect(container.querySelector("p")?.textContent).toContain("<b>hi</b>");
  });

  it("partial is muted and marked", () => {
    const { container } = render(<TranscriptLine startMs={0} speaker={speaker} words={wordsFromText("một hai ba bốn")} partial />);
    expect(container.querySelector("[data-state]")?.getAttribute("data-state")).toBe("partial");
    expect(container.querySelector("p")?.className).toContain("text-muted");
  });

  it("provisional speaker shows the identifying copy", () => {
    const { container } = render(<TranscriptLine startMs={0} speaker={null} words={wordsFromText("xin chào")} />);
    expect(screen.getByText("Identifying speaker…")).toBeTruthy();
    expect(container.querySelector('[data-kind="unknown"]')).toBeTruthy();
  });

  it("low-confidence words carry an accessible hint", () => {
    const { container } = render(<TranscriptLine startMs={0} speaker={speaker} words={wordsFromText("sang CoreML.", "CoreML.")} />);
    const low = container.querySelector('[data-low="true"]');
    expect(low?.textContent).toContain("CoreML.");
    expect(low?.textContent).toContain("Low confidence");
    expect(container.querySelectorAll('[data-low="true"]').length).toBe(1);
  });

  it("overlap: a marker with its hint, the text muted", () => {
    const { container } = render(<TranscriptLine startMs={0} speaker={speaker} words={wordsFromText("hai người cùng nói")} overlap />);
    const tag = screen.getByTestId("overlap-tag");
    expect(tag.textContent).toContain("Talking over each other");
    expect(tag.getAttribute("title")).toBe("Two people spoke at once here, so some words may be wrong.");
    expect(container.querySelector("p")?.className).toContain("text-muted");
    expect(container.querySelector("[data-overlap]")?.getAttribute("data-overlap")).toBe("true");
  });

  it("inside a stack the line keeps the short label only", () => {
    render(<TranscriptLine startMs={0} speaker={speaker} words={wordsFromText("x")} overlap overlapHint={false} />);
    const tag = screen.getByTestId("overlap-tag");
    expect(tag.textContent).toBe("Talking over each other");
    expect(tag.getAttribute("title")).toBeNull();
  });

  it("no overlap, no marker", () => {
    render(<TranscriptLine startMs={0} speaker={speaker} words={wordsFromText("x")} />);
    expect(screen.queryByTestId("overlap-tag")).toBeNull();
  });

  it("marked shows a named star; edited shows its label", () => {
    render(<TranscriptLine startMs={26_000} speaker={speaker} words={wordsFromText("x")} marked edited />);
    expect(screen.getByRole("img", { name: "Marked 0:26" })).toBeTruthy();
    expect(screen.getByText("Edited")).toBeTruthy();
  });

  it("playing highlights only the active word", () => {
    const { container } = render(<TranscriptLine startMs={0} speaker={speaker} words={wordsFromText("Agreed. Live rename phải")} playing activeWordIndex={2} />);
    const active = container.querySelectorAll('[data-active="true"]');
    expect(active.length).toBe(1);
    expect(active[0]?.textContent).toBe("rename");
    expect(container.querySelector("[data-playing]")?.getAttribute("aria-current")).toBe("true");
  });

  it("selected is exposed as data state", () => {
    const { container } = render(<TranscriptLine startMs={0} speaker={speaker} words={wordsFromText("x")} selected />);
    expect(container.querySelector("[data-selected]")).toBeTruthy();
  });

  it("hover actions are real buttons and the time plays", async () => {
    const onEdit = vi.fn();
    const onChange = vi.fn();
    const onPlay = vi.fn();
    render(<TranscriptLine startMs={65_000} speaker={speaker} words={wordsFromText("x")} onEdit={onEdit} onChangeSpeaker={onChange} onPlay={onPlay} />);
    await userEvent.click(screen.getByRole("button", { name: "Edit text" }));
    await userEvent.click(screen.getByRole("button", { name: "Change speaker" }));
    await userEvent.click(screen.getByRole("button", { name: "Play from 1:05" }));
    expect([onEdit, onChange, onPlay].map((f) => f.mock.calls.length)).toEqual([1, 1, 1]);
  });
});
