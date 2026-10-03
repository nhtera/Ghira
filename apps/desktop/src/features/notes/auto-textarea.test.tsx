// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AutoTextarea } from "./auto-textarea";

afterEach(cleanup);

describe("AutoTextarea", () => {
  it("keeps the same textarea (focus, caret, text) when a trailing chip appears while typing", () => {
    const onCommit = vi.fn();
    const props = { value: "Voice profiles", label: "Note", onCommit, keepOnEmpty: true };
    const { rerender } = render(<AutoTextarea {...props} />);
    const box = screen.getByRole("textbox", { name: "Note" }) as HTMLTextAreaElement;
    box.focus();
    fireEvent.change(box, { target: { value: "Voice profiles are saved" } });
    rerender(<AutoTextarea {...props} trailing={<button type="button">00:12</button>} />);
    const after = screen.getByRole("textbox", { name: "Note" }) as HTMLTextAreaElement;
    expect(after).toBe(box);
    expect(document.activeElement).toBe(box);
    expect(box.value).toBe("Voice profiles are saved");
    expect(screen.getByRole("button", { name: "00:12" })).toBeTruthy();
    rerender(<AutoTextarea {...props} />);
    expect(screen.getByRole("textbox", { name: "Note" })).toBe(box);
  });
});
