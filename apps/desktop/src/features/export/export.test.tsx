// SPDX-License-Identifier: Apache-2.0
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider, ToastProvider } from "@ghi/ui";
import { changeLanguage } from "i18next";
import { ipc } from "../../ipc";
import { canExport, exportContent } from "./export-options";
import { ExportSheet } from "./export-sheet";

afterEach(async () => {
  cleanup();
  vi.restoreAllMocks();
  await changeLanguage("en");
});

describe("export options", () => {
  it("subtitles are transcript-only whatever was ticked", () => {
    expect(exportContent("srt", true, false, "en")).toEqual({ notes: false, transcript: true, vietnamese: false });
    expect(exportContent("markdown", true, false, "vi")).toEqual({ notes: true, transcript: false, vietnamese: true });
  });
  it("needs a part ticked unless it is subtitles", () => {
    expect(canExport("docx", false, false)).toBe(false);
    expect(canExport("vtt", false, false)).toBe(true);
  });
});

function sheet(meetings: string[]) {
  const onOpenChange = vi.fn();
  render(
    <QueryClientProvider client={new QueryClient()}>
      <PlatformProvider value="mac">
        <ToastProvider label="toasts">
          <ExportSheet open onOpenChange={onOpenChange} meetings={meetings} />
        </ToastProvider>
      </PlatformProvider>
    </QueryClientProvider>,
  );
  return onOpenChange;
}

describe("ExportSheet", () => {
  it("one meeting: the chosen format and parts go to exportMeeting; Saved toast with Show in Finder", async () => {
    const exp = vi.spyOn(ipc.commands, "exportMeeting").mockResolvedValue({ status: "ok", data: "Standup.docx" });
    const reveal = vi.spyOn(ipc.commands, "revealLastExport").mockResolvedValue({ status: "ok", data: null });
    const close = sheet(["m1"]);
    fireEvent.click(screen.getByRole("radio", { name: "Word document (.docx)" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Transcript" }));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Export…" })));
    expect(exp).toHaveBeenCalledWith("m1", "docx", { notes: true, transcript: true, vietnamese: false });
    expect(close).toHaveBeenCalledWith(false);
    expect(await screen.findByText("Saved Standup.docx")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Show in Finder", hidden: true }));
    expect(reveal).toHaveBeenCalled();
  });

  it("cancelling the save dialog (null) shows no toast and keeps the sheet open", async () => {
    vi.spyOn(ipc.commands, "exportMeeting").mockResolvedValue({ status: "ok", data: null });
    const close = sheet(["m1"]);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Export…" })));
    expect(close).not.toHaveBeenCalled();
    expect(screen.queryByText(/Saved/)).toBeNull();
  });

  it("several meetings go to exportMeetings with the sheet title for the folder dialog; subtitles lock the parts", async () => {
    const many = vi.spyOn(ipc.commands, "exportMeetings").mockResolvedValue({ status: "ok", data: 3 });
    sheet(["a", "b", "c"]);
    fireEvent.click(screen.getByRole("radio", { name: "Subtitles (.srt)" }));
    expect((screen.getByRole("checkbox", { name: "Notes" }) as HTMLButtonElement).disabled).toBe(true);
    // Copy / Obsidian are for a single meeting.
    expect(screen.queryByRole("button", { name: "Copy as text" })).toBeNull();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Export…" })));
    expect(many).toHaveBeenCalledWith(["a", "b", "c"], "srt", { notes: false, transcript: true, vietnamese: false }, "Export notes");
    expect(await screen.findByText("Saved 3 meetings")).toBeTruthy();
  });

  it("copy as Markdown writes meetingAsText to the clipboard; Obsidian asks for the folder only on 'Change folder…'", async () => {
    vi.spyOn(ipc.commands, "meetingAsText").mockResolvedValue({ status: "ok", data: "# Standup" });
    const write = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    const obs = vi.spyOn(ipc.commands, "exportObsidian").mockResolvedValue({ status: "ok", data: "Standup.md" });
    sheet(["m1"]);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy as Markdown" })));
    expect(ipc.commands.meetingAsText).toHaveBeenCalledWith("m1", true, { notes: true, transcript: false, vietnamese: false });
    expect(write).toHaveBeenCalledWith("# Standup");
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Export to Obsidian…" })));
    expect(obs).toHaveBeenLastCalledWith("m1", expect.objectContaining({ notes: true }), false, expect.any(String));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Change folder…" })));
    expect(obs).toHaveBeenLastCalledWith("m1", expect.anything(), true, expect.any(String));
  });

  it("says where the file goes (the remembered folder's name) and Change… picks another", async () => {
    const choose = vi.spyOn(ipc.commands, "chooseExportFolder").mockResolvedValue({ status: "ok", data: "Notes" });
    sheet(["m1"]);
    const line = await screen.findByTestId("export-destination");
    await vi.waitFor(() => expect(line.textContent).toMatch(/Documents/));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: /^(Change…|export\.changeDestination)$/ })));
    expect(choose).toHaveBeenCalled();
    await vi.waitFor(() => expect(line.textContent).toMatch(/Notes/));
  });

  it("headings follow the app language", async () => {
    await changeLanguage("vi");
    const exp = vi.spyOn(ipc.commands, "exportMeeting").mockResolvedValue({ status: "ok", data: "x.md" });
    sheet(["m1"]);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Xuất…" })));
    expect(exp).toHaveBeenCalledWith("m1", "markdown", { notes: true, transcript: false, vietnamese: true });
  });
});
