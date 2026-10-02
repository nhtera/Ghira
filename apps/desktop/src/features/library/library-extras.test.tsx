// SPDX-License-Identifier: Apache-2.0
// D3 additions: filters, multi-select, virtualization, delete with Undo.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, renderHook, screen, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider, ToastProvider } from "@ghi/ui";
import type { MeetingRow } from "../../bindings";
import { ipc } from "../../ipc";
import { NO_FILTERS, applyFilters, dateFrom, searchParams, sourceOf } from "./filters";
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
  ...over,
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("filters", () => {
  const rows = [
    row("a", { people: [{ name: "Linh", colorSlot: 2 }], template: "standup" }),
    row("b", { source: "import", startedAt: at(10) }),
    row("c", {
      people: [
        { name: "Minh", colorSlot: 3 },
        { name: "Linh", colorSlot: 2 },
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
    });
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
