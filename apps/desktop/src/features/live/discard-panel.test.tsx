// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const commands = vi.hoisted(() => ({ discardPreview: vi.fn(), discardLast: vi.fn(), discardFrom: vi.fn() }));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { DiscardPanel } from "./discard-panel";
import { renderLive } from "./test-utils";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const preview = { fromMs: 1000, lines: ["first line", "second line"], notes: ["a note"], marks: 2 };

describe("DiscardPanel", () => {
  it("lists what goes before confirming, then discards", async () => {
    commands.discardPreview.mockImplementation(() => ok(preview));
    commands.discardFrom.mockImplementation(() => ok(1000));
    const onClose = vi.fn();
    renderLive(<DiscardPanel meeting="m1" seconds={300} onClose={onClose} />);
    expect(await screen.findByText(/Discard the last 5 minutes/)).toBeTruthy();
    expect(screen.getByText(/Removes the audio and 2 transcript lines, 1 note, 2 marks/)).toBeTruthy();
    expect(screen.getByText("first line")).toBeTruthy();
    expect(commands.discardPreview).toHaveBeenCalledWith(300);
    expect(commands.discardFrom).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Discard" }));
    // Cuts at the previewed span, not "the last N seconds from now".
    await waitFor(() => expect(commands.discardFrom).toHaveBeenCalledWith(1000));
    expect(commands.discardLast).not.toHaveBeenCalled();
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("cancel leaves everything alone (and focus starts on Cancel)", async () => {
    commands.discardPreview.mockImplementation(() => ok(preview));
    const onClose = vi.fn();
    renderLive(<DiscardPanel meeting="m1" seconds={60} onClose={onClose} />);
    const cancel = await screen.findByRole("button", { name: "Cancel" });
    expect(document.activeElement).toBe(cancel);
    await userEvent.click(cancel);
    expect(onClose).toHaveBeenCalled();
    expect(commands.discardLast).not.toHaveBeenCalled();
  });

  it("is not a modal dialog", async () => {
    commands.discardPreview.mockImplementation(() => ok(preview));
    renderLive(<DiscardPanel meeting="m1" seconds={60} onClose={() => {}} />);
    await screen.findByRole("alertdialog");
    expect(document.querySelector("[aria-modal=true]")).toBeNull();
  });
});
