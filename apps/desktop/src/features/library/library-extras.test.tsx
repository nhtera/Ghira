// SPDX-License-Identifier: Apache-2.0
// D3 additions: filters, multi-select, virtualization, delete with Undo.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, renderHook, screen, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider, ToastProvider } from "@ghi/ui";
import type { MeetingRow } from "../../bindings";
import { ipc } from "../../ipc";
import { NO_FILTERS, applyFilters, dateFrom, hasFilters, searchParams, sourceOf } from "./filters";
import { LibraryList } from "./library-list";
import { rangeIds } from "./selection";
import { UNDO_MS, usePendingDelete } from "./use-pending-delete";

const NOW = new Date(2026, 9, 1, 15, 0);
const at = (daysAgo: number) => new Date(2026, 9, 1 - daysAgo, 9, 30).getTime();
const row = (gid: string, over: Partial<MeetingRow> = {}): MeetingRow => ({
  gid,
  title: `Meeting ${gid}`,
  startedAt: at(0),
  durationMs: 60_000,
  source: "live",
  mode: "call",
  status: "ready",
  transcriptVersion: 2,
  cloudUsed: false,
  consentConfirmed: false,
  template: null,
  people: [],
  job: null,
  folder: null,
  tags: [],
  sourceApp: null,
  summary: null,
  unnamedVoices: 0,
  ...over,
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("filters", () => {
  const rows = [
    row("a", { people: [{ name: "Linh", colorSlot: 2, isMe: false }], template: "standup" }),
    row("b", { source: "import", startedAt: at(10) }),
    row("c", {
      people: [
        { name: "Minh", colorSlot: 3, isMe: false },
        { name: "Linh", colorSlot: 2, isMe: false },
      ],
      startedAt: at(3),
    }),
  ];
  const ids = (f: Partial<typeof NO_FILTERS>) => applyFilters(rows, { ...NO_FILTERS, ...f }, NOW).map((r) => r.gid);
  it("no filters keeps everything", () => expect(ids({})).toEqual(["a", "b", "c"]));
  it("people: any selected person", () => expect(ids({ people: ["Linh"] })).toEqual(["a", "c"]));
  it("source: file vs live (import counts as file)", () => {
    expect(ids({ source: ["file"] })).toEqual(["b"]);
    expect(ids({ source: ["live"] })).toEqual(["a", "c"]);
    expect(sourceOf({ source: "file" })).toBe("file");
  });
  it("template: null means general", () => {
    expect(ids({ template: ["general"] })).toEqual(["b", "c"]);
    expect(ids({ template: ["standup"] })).toEqual(["a"]);
  });
  it("date presets cut at calendar days", () => {
    expect(ids({ date: "today" })).toEqual(["a"]);
    expect(ids({ date: "week" })).toEqual(["a", "c"]);
    expect(ids({ date: "month" })).toEqual(["a", "b", "c"]);
    expect(dateFrom("today", NOW)).toBe(new Date(2026, 9, 1).getTime());
  });
  it("search params pass single choices only", () => {
    expect(searchParams({ ...NO_FILTERS, source: ["live"], template: ["a", "b"], date: "today" }, NOW)).toEqual({
      source: "live",
      template: null,
      fromMs: new Date(2026, 9, 1).getTime(),
      toMs: null,
      folder: null,
      tags: null,
    });
  });
  it("folder: one folder, or \"\" for no folder", () => {
    const withFolders = [row("a", { folder: "f1" }), row("b"), row("c", { folder: "f2" })];
    const pick = (folder: string | null) => applyFilters(withFolders, { ...NO_FILTERS, folder }, NOW).map((r) => r.gid);
    expect(pick("f1")).toEqual(["a"]);
    expect(pick("")).toEqual(["b"]);
    expect(pick(null)).toEqual(["a", "b", "c"]);
  });
  it("tags: any of the selected tags, combined with the other filters", () => {
    const tagged = [
      row("a", { tags: [{ gid: "t1", name: "Họp" }], folder: "f1" }),
      row("b", { tags: [{ gid: "t2", name: "Hộp" }], folder: "f1" }),
      row("c", { tags: [{ gid: "t1", name: "Họp" }, { gid: "t2", name: "Hộp" }] }),
      row("d"),
    ];
    const pick = (f: Partial<typeof NO_FILTERS>) => applyFilters(tagged, { ...NO_FILTERS, ...f }, NOW).map((r) => r.gid);
    expect(pick({ tags: ["t1"] })).toEqual(["a", "c"]);
    expect(pick({ tags: ["t1", "t2"] })).toEqual(["a", "b", "c"]);
    expect(pick({ tags: ["t2"], folder: "f1" })).toEqual(["b"]);
    expect(pick({ tags: ["t1"], folder: "" })).toEqual(["c"]);
  });
  it("search params carry the folder and any-of tags to the store", () => {
    expect(searchParams({ ...NO_FILTERS, folder: "", tags: ["t1", "t2"] }, NOW)).toMatchObject({ folder: "", tags: ["t1", "t2"] });
    expect(searchParams({ ...NO_FILTERS, folder: "f9" }, NOW)).toMatchObject({ folder: "f9", tags: null });
  });
  it("a folder or a tag counts as an active filter", () => {
    expect(hasFilters({ ...NO_FILTERS, folder: "" })).toBe(true);
    expect(hasFilters({ ...NO_FILTERS, tags: ["t1"] })).toBe(true);
    expect(hasFilters(NO_FILTERS)).toBe(false);
  });
});

describe("rangeIds", () => {
  const order = ["a", "b", "c", "d", "e"];
  it("selects the run from the anchor, either direction", () => {
    expect(rangeIds(order, "b", "d")).toEqual(["b", "c", "d"]);
    expect(rangeIds(order, "d", "b")).toEqual(["b", "c", "d"]);
  });
  it("falls back to the clicked row without an anchor", () => expect(rangeIds(order, null, "c")).toEqual(["c"]));
});

function Harness({ rows, onSel }: { rows: MeetingRow[]; onSel?: (s: Set<string>) => void }) {
  const [sel, setSel] = useState<ReadonlySet<string>>(new Set());
  return (
    <PlatformProvider value="mac">
      <LibraryList
        now={NOW}
        rows={rows}
        onOpen={() => {}}
        selected={sel}
        onSelectionChange={(s) => {
          setSel(s);
          onSel?.(s);
        }}
      />
    </PlatformProvider>
  );
}

describe("LibraryList selection", () => {
  it("click toggles, shift-click selects the range", () => {
    let last = new Set<string>();
    render(<Harness rows={["a", "b", "c", "d"].map((g) => row(g))} onSel={(s) => (last = s)} />);
    const box = (g: string) => screen.getByRole("checkbox", { name: `Select Meeting ${g}` });
    fireEvent.click(box("a"));
    expect([...last]).toEqual(["a"]);
    fireEvent.click(box("d"), { shiftKey: true });
    expect([...last].sort()).toEqual(["a", "b", "c", "d"]);
    fireEvent.click(box("b"));
    expect([...last].sort()).toEqual(["a", "c", "d"]);
    expect(box("a").getAttribute("aria-checked")).toBe("true");
  });

  it("virtualizes: 1,200 rows render only a window", () => {
    render(<Harness rows={Array.from({ length: 1200 }, (_, i) => row(`m${i}`, { startedAt: at(0) - i * 60_000 }))} />);
    const n = screen.getAllByRole("listitem").length;
    expect(n).toBeGreaterThan(0);
    expect(n).toBeLessThan(60);
  });
});

describe("usePendingDelete", () => {
  function setup() {
    const client = new QueryClient();
    return renderHook(() => usePendingDelete(), {
      wrapper: ({ children }) => (
        <QueryClientProvider client={client}>
          <ToastProvider label="toasts">{children}</ToastProvider>
        </QueryClientProvider>
      ),
    });
  }
  beforeEach(() => vi.useFakeTimers());

  it("hides at once, deletes after the delay", async () => {
    const del = vi.spyOn(ipc.commands, "deleteMeeting").mockResolvedValue({ status: "ok", data: null });
    const { result } = setup();
    act(() => result.current.schedule(["a", "b"]));
    expect([...result.current.hidden]).toEqual(["a", "b"]);
    await act(() => vi.advanceTimersByTimeAsync(UNDO_MS - 100));
    expect(del).not.toHaveBeenCalled();
    await act(() => vi.advanceTimersByTimeAsync(200));
    expect(del.mock.calls.map((c) => c[0])).toEqual(["a", "b"]);
  });

  it("Undo cancels the delete and shows the rows again", async () => {
    const del = vi.spyOn(ipc.commands, "deleteMeeting").mockResolvedValue({ status: "ok", data: null });
    const { result } = setup();
    act(() => result.current.schedule(["a"]));
    // The toast's Undo button.
    fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(result.current.hidden.size).toBe(0);
    await act(() => vi.advanceTimersByTimeAsync(UNDO_MS * 2));
    expect(del).not.toHaveBeenCalled();
  });

  it("Undo after the delete already ran says it is too late and does not bring the row back", async () => {
    vi.spyOn(ipc.commands, "deleteMeeting").mockResolvedValue({ status: "ok", data: null });
    const { result } = setup();
    act(() => result.current.schedule(["a"]));
    const undo = screen.getByRole("button", { name: "Undo" });
    // Hovering the toast pauses Radix's close timer, which is how Undo outlives the delete timer.
    fireEvent.focus(document.querySelector("ol")!);
    await act(() => vi.advanceTimersByTimeAsync(UNDO_MS + 100));
    fireEvent.click(undo);
    expect(screen.getByText(/Too late to undo/)).toBeTruthy();
  });

  it("a refused delete (a job runs) brings the row back and says why", async () => {
    vi.spyOn(ipc.commands, "deleteMeeting").mockResolvedValue({ status: "error", error: "busy" });
    const { result } = setup();
    act(() => result.current.schedule(["a"]));
    await act(() => vi.advanceTimersByTimeAsync(UNDO_MS + 100));
    expect(result.current.hidden.size).toBe(0);
    expect(screen.getByText(/busy/)).toBeTruthy();
  });
});

// Keep `within` imported for parity with the other library tests.
void within;
