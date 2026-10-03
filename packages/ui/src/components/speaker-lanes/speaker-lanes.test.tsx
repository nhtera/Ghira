// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { overlapsOf, SpeakerLanes } from "./speaker-lanes";

afterEach(cleanup);

const speakers = [
  { id: 0, label: "Me", colorSlot: 1 },
  { id: 1, label: "Linh", colorSlot: 2 },
];
const segments = [
  { speaker: 0, t0Ms: 0, t1Ms: 40_000 },
  { speaker: 1, t0Ms: 30_000, t1Ms: 60_000 },
];

describe("overlapsOf", () => {
  it("finds the stretch where two speakers talk at once, for both", () => {
    const o = overlapsOf(segments);
    expect(o.get(0)).toEqual([[30_000, 40_000]]);
    expect(o.get(1)).toEqual([[30_000, 40_000]]);
    expect(overlapsOf([{ speaker: 0, t0Ms: 0, t1Ms: 10 }, { speaker: 0, t0Ms: 5, t1Ms: 20 }]).size).toBe(0);
  });
});

describe("SpeakerLanes", () => {
  it("renders a lane per speaker with its segments in the speaker color", () => {
    const { container } = render(<SpeakerLanes speakers={speakers} segments={segments} durationMs={60_000} />);
    expect(container.querySelectorAll("[data-lane]").length).toBe(2);
    const first = container.querySelector('[data-lane="0"] i') as HTMLElement;
    expect(first.style.background).toContain("--s1");
    expect(parseFloat(first.style.width)).toBeCloseTo(66.67, 1);
    expect(screen.getByText("Linh")).toBeTruthy();
  });

  it("hatches overlap regions", () => {
    const { container } = render(<SpeakerLanes speakers={speakers} segments={segments} durationMs={60_000} />);
    expect(container.querySelectorAll('[data-overlap="true"]').length).toBe(2);
  });

  it("uses a neutral fill for the Others lane", () => {
    const { container } = render(<SpeakerLanes speakers={[{ id: 9, label: "Others · 3", colorSlot: 0 }]} segments={[{ speaker: 9, t0Ms: 0, t1Ms: 5000 }]} durationMs={10_000} />);
    expect((container.querySelector('[data-lane="9"] i') as HTMLElement).style.background).toContain("--muted");
  });

  it("live: the newest segment fades into the edge", () => {
    const { container } = render(<SpeakerLanes live speakers={speakers} segments={[{ speaker: 0, t0Ms: 0, t1Ms: 60_000 }]} durationMs={60_000} />);
    expect((container.querySelector('[data-lane="0"] i') as HTMLElement).style.background).toContain("linear-gradient");
  });

  it("is a focusable slider that moves with the arrow keys and seeks", async () => {
    const onSeek = vi.fn();
    render(<SpeakerLanes speakers={speakers} segments={segments} durationMs={60_000} onSeek={onSeek} />);
    const slider = screen.getByRole("slider", { name: "Timeline" });
    expect(slider.getAttribute("aria-valuemax")).toBe("60000");
    await userEvent.tab();
    expect(document.activeElement).toBe(slider);
    await userEvent.keyboard("{ArrowRight}{ArrowRight}");
    expect(onSeek).toHaveBeenLastCalledWith(10_000);
    expect(slider.getAttribute("aria-valuetext")).toBe("00:10");
    await userEvent.keyboard("{Shift>}{ArrowRight}{/Shift}");
    expect(onSeek).toHaveBeenLastCalledWith(40_000);
    await userEvent.keyboard("{Home}");
    expect(onSeek).toHaveBeenLastCalledWith(0);
    await userEvent.keyboard("{End}");
    expect(onSeek).toHaveBeenLastCalledWith(60_000);
  });

  it("hover shows the time under the pointer", () => {
    const { container } = render(<SpeakerLanes speakers={speakers} segments={segments} durationMs={60_000} />);
    const slider = screen.getByRole("slider");
    slider.getBoundingClientRect = () => ({ left: 0, width: 200, top: 0, right: 200, bottom: 10, height: 10, x: 0, y: 0, toJSON: () => ({}) });
    fireEvent.pointerMove(slider, { clientX: 100 });
    expect(container.textContent).toContain("00:30");
    fireEvent.pointerLeave(slider);
    expect(container.querySelector("[data-marker]")).toBeNull();
  });
});
