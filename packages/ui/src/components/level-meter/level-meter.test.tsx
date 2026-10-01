// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LevelMeter, levelState } from "./level-meter";

afterEach(cleanup);

describe("levelState", () => {
  it("classifies dBFS", () => {
    expect(levelState(-58)).toBe("silent");
    expect(levelState(-22)).toBe("normal");
    expect(levelState(-0.3)).toBe("clipping");
    expect(levelState(null)).toBe("noDevice");
  });
});

describe("LevelMeter", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("is a named meter with a value range", () => {
    render(<LevelMeter source="mic" db={-22} />);
    const m = screen.getByRole("meter", { name: "Mic" });
    expect(m.getAttribute("aria-valuemin")).toBe("-60");
    expect(m.getAttribute("aria-valuemax")).toBe("0");
    expect(m.getAttribute("aria-valuenow")).toBe("-22");
  });

  it.each([
    [-58, "silent", "No speech"],
    [-0.3, "clipping", "Too loud"],
    [null, "noDevice", "No microphone"],
  ] as const)("db %s shows a text note, not only color", (db, state, note) => {
    const { container } = render(<LevelMeter source="mic" db={db} />);
    expect((container.firstChild as HTMLElement).dataset.state).toBe(state);
    expect(screen.getByText(note)).toBeTruthy();
  });

  it("no device sits at the floor and says why", () => {
    render(<LevelMeter source="system" db={null} />);
    const m = screen.getByRole("meter", { name: "System" });
    // A meter needs a value (axe aria-required-attr); the text carries the state.
    expect(m.getAttribute("aria-valuenow")).toBe(m.getAttribute("aria-valuemin"));
    expect(m.getAttribute("aria-valuetext")).toBe("No system audio");
    expect(screen.getByText("No system audio")).toBeTruthy();
  });

  it("freezes to the state width under reduced motion", () => {
    vi.stubGlobal("matchMedia", () => ({ matches: true, addEventListener() {}, removeEventListener() {} }));
    const { container, rerender } = render(<LevelMeter source="mic" db={-40} />);
    const bar = () => container.querySelector("i") as HTMLElement;
    expect(bar().style.width).toBe("62%");
    rerender(<LevelMeter source="mic" db={-15} />);
    expect(bar().style.width).toBe("62%");
  });
});
