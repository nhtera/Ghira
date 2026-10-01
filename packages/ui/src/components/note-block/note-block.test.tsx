// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NoteBlock, type NoteKind } from "./note-block";

afterEach(cleanup);

const PROVENANCE: Record<NoteKind, string> = {
  user: "You wrote",
  ai: "Written by Ghira from the transcript",
  edited: "Edited by you · kept on regenerate",
  missing: "Not found in the transcript. Kept as your note.",
};

describe("NoteBlock", () => {
  it.each(Object.entries(PROVENANCE))("%s shows its provenance as icon + text", (kind, text) => {
    const { container } = render(<NoteBlock kind={kind as NoteKind} text="beta scope nov" />);
    expect(screen.getByText(text)).toBeTruthy();
    expect(container.querySelector("svg")).toBeTruthy();
    expect(container.querySelector("[data-kind]")?.getAttribute("data-kind")).toBe(kind);
  });

  it("renders the note as plain text (RT-6)", () => {
    const { container } = render(<NoteBlock kind="ai" text={"<b>bold</b> [link](http://x)"} />);
    expect(container.querySelector("b")).toBeNull();
    expect(container.querySelector("a")).toBeNull();
    expect(container.querySelector("p")?.textContent).toContain("<b>bold</b> [link](http://x)");
  });

  it("AI text uses the muted AI tone, user text is bold ink", () => {
    const { container, rerender } = render(<NoteBlock kind="ai" text="x" />);
    expect(container.querySelector("p")?.className).toContain("text-ai");
    rerender(<NoteBlock kind="user" text="x" />);
    expect(container.querySelector("p")?.className).toContain("font-bold");
  });

  it("citations are chips that report which one was activated", async () => {
    const onCite = vi.fn();
    render(<NoteBlock kind="ai" text="x" citations={[{ timeMs: 65_000 }, { timeMs: 120_000 }]} onCite={onCite} />);
    await userEvent.click(screen.getByRole("button", { name: "Show in transcript 2:00" }));
    expect(onCite).toHaveBeenCalledWith(1, { timeMs: 120_000 });
  });
});
