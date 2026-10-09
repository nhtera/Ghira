// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MeetingDetail } from "../../bindings";

const commands = vi.hoisted(() => ({ retranscribe: vi.fn() }));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { renderLive } from "../live/test-utils";
import { RetranscribePanel, initialLanguage } from "./retranscribe-panel";

const detail = (language: string | null) => ({ gid: "m1", language }) as MeetingDetail;

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("RetranscribePanel", () => {
  it("starts from the meeting's language", () => {
    expect(initialLanguage(detail("vi"))).toBe("vi");
    expect(initialLanguage(detail("mixed"))).toBe("auto");
    expect(initialLanguage(detail(null))).toBe("auto");
  });

  it("transcribes again in the chosen language, then closes", async () => {
    commands.retranscribe.mockResolvedValue({ status: "ok", data: false });
    const onClose = vi.fn();
    renderLive(<RetranscribePanel detail={detail("vi")} onClose={onClose} />);
    expect(screen.getByRole("radio", { name: "Tiếng Việt" }).getAttribute("aria-checked")).toBe("true");
    // Focus starts on Cancel, the safe default.
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel" }));
    await userEvent.click(screen.getByRole("radio", { name: "English" }));
    await userEvent.click(screen.getByRole("button", { name: "Transcribe again" }));
    expect(commands.retranscribe).toHaveBeenCalledWith("m1", "en");
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(await screen.findByText("Transcribing again…")).toBeTruthy();
  });

  it("stays open and says why when it is refused; Escape closes", async () => {
    commands.retranscribe.mockResolvedValue({ status: "error", error: "the meeting is still being processed" });
    const onClose = vi.fn();
    renderLive(<RetranscribePanel detail={detail(null)} onClose={onClose} />);
    await userEvent.click(screen.getByRole("button", { name: "Transcribe again" }));
    expect(commands.retranscribe).toHaveBeenCalledWith("m1", "auto");
    // The toast is announced by a second, screen-reader-only copy (role=status) for a moment: look at the visible one.
    expect(await screen.findByText(/still being processed/, { ignore: "[role=status]" })).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });
});
