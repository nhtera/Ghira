// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NoteLine } from "../../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const commands = vi.hoisted(() => ({
  noteLines: vi.fn(),
  addNoteLine: vi.fn(),
  updateNoteLine: vi.fn(),
  deleteNoteLine: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { Notepad } from "./notepad";
import { renderLive, setLive } from "./test-utils";

let seq = 0;
beforeEach(() => {
  seq = 0;
  setLive({ meeting: "m1", state: "recording", startedAtMs: Date.now() - 5000 });
  commands.noteLines.mockImplementation(() => ok<NoteLine[]>([]));
  commands.addNoteLine.mockImplementation((_m: string, text: string, tMs: number | null, kind: string) => ok<NoteLine>({ gid: `n${++seq}`, text, tMs, kind }));
  commands.updateNoteLine.mockImplementation(() => ok(null));
  commands.deleteNoteLine.mockImplementation(() => ok(null));
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("Notepad", () => {
  it("saves a typed line with the meeting time as its anchor", async () => {
    const user = userEvent.setup();
    renderLive(<Notepad meeting="m1" />);
    await user.type(screen.getByLabelText("Add a note"), "ship the beta{Enter}");
    await waitFor(() => expect(commands.addNoteLine).toHaveBeenCalledTimes(1));
    const [meeting, text, tMs, kind] = commands.addNoteLine.mock.calls[0];
    expect([meeting, text, kind]).toEqual(["m1", "ship the beta", "note"]);
    expect(tMs).toBeGreaterThanOrEqual(5000);
    expect(await screen.findByText("ship the beta")).toBeTruthy();
    expect((screen.getByLabelText("Add a note") as HTMLInputElement).value).toBe("");
  });

  it("ignores an empty line", async () => {
    const user = userEvent.setup();
    renderLive(<Notepad meeting="m1" />);
    await user.type(screen.getByLabelText("Add a note"), "   {Enter}");
    expect(commands.addNoteLine).not.toHaveBeenCalled();
  });

  it("tags the typed line when a tag is clicked", async () => {
    const user = userEvent.setup();
    renderLive(<Notepad meeting="m1" />);
    await user.type(screen.getByLabelText("Add a note"), "go with option B");
    await user.click(screen.getByRole("button", { name: "Decision" }));
    await waitFor(() => expect(commands.addNoteLine).toHaveBeenCalled());
    expect(commands.addNoteLine.mock.calls[0][3]).toBe("decision");
  });

  it("a tag picked first applies to the next line, then resets", async () => {
    const user = userEvent.setup();
    renderLive(<Notepad meeting="m1" />);
    const tag = screen.getByRole("button", { name: "Action" });
    await user.click(tag);
    expect(tag.getAttribute("aria-pressed")).toBe("true");
    await user.type(screen.getByLabelText("Add a note"), "send the deck{Enter}");
    await waitFor(() => expect(commands.addNoteLine).toHaveBeenCalled());
    expect(commands.addNoteLine.mock.calls[0][3]).toBe("action");
    expect(tag.getAttribute("aria-pressed")).toBe("false");
  });

  it("Alt+digit tags the line", async () => {
    const user = userEvent.setup();
    renderLive(<Notepad meeting="m1" />);
    await user.type(screen.getByLabelText("Add a note"), "who owns it");
    await user.keyboard("{Alt>}3{/Alt}");
    await waitFor(() => expect(commands.addNoteLine).toHaveBeenCalled());
    expect(commands.addNoteLine.mock.calls[0][3]).toBe("question");
  });

  it("edits and deletes a saved line", async () => {
    const user = userEvent.setup();
    commands.noteLines.mockImplementation(() => ok<NoteLine[]>([{ gid: "n9", text: "old text", tMs: 1000, kind: "note" }]));
    renderLive(<Notepad meeting="m1" />);
    await screen.findByText("old text");
    await user.click(screen.getByRole("button", { name: "Edit text" }));
    const field = screen.getByRole("textbox", { name: "Edit text" });
    await user.clear(field);
    await user.type(field, "new text{Enter}");
    await waitFor(() => expect(commands.updateNoteLine).toHaveBeenCalledWith("m1", "n9", "new text"));
    expect(await screen.findByText("new text")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(commands.deleteNoteLine).toHaveBeenCalledWith("m1", "n9"));
    await waitFor(() => expect(screen.queryByText("new text")).toBeNull());
    // Undo puts it back with the same time and tag.
    await user.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() => expect(commands.addNoteLine).toHaveBeenCalledWith("m1", "new text", 1000, "note"));
    expect(await screen.findByText("new text")).toBeTruthy();
  });

  it("anchors the line at its first keystroke, not at Enter", async () => {
    const user = userEvent.setup();
    renderLive(<Notepad meeting="m1" />);
    const input = screen.getByLabelText("Add a note");
    await user.type(input, "a");
    const at = Date.now();
    await new Promise((r) => setTimeout(r, 300));
    await user.type(input, "bc{Enter}");
    await waitFor(() => expect(commands.addNoteLine).toHaveBeenCalled());
    const tMs = commands.addNoteLine.mock.calls[0][2] as number;
    // Meeting time at the first key: the clock started 5 s before, so ~5000 + (key - start).
    expect(tMs).toBeLessThan(5000 + (Date.now() - at) + 100 - 200);
  });

  it("shows markup as text", async () => {
    commands.noteLines.mockImplementation(() => ok<NoteLine[]>([{ gid: "n1", text: "<b>x</b>", tMs: 0, kind: "note" }]));
    const { container } = renderLive(<Notepad meeting="m1" />);
    await screen.findByText("<b>x</b>");
    expect(container.querySelector("b")).toBeNull();
  });
});
