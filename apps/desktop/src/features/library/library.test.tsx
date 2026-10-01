// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider } from "@ghi/ui";
import type { MeetingRow } from "../../bindings";
import { groupByDay } from "./group-by-day";
import { LibraryList } from "./library-list";
import { rowStatus } from "./meeting-status";

const NOW = new Date(2026, 9, 1, 15, 0); // Thu 1 Oct 2026
const at = (daysAgo: number, h = 9) => new Date(2026, 9, 1 - daysAgo, h, 30).getTime();
const row = (gid: string, over: Partial<MeetingRow> = {}): MeetingRow => ({
  gid,
  title: `Meeting ${gid}`,
  startedAt: at(0),
  durationMs: 125_000,
  source: "call",
  mode: "call",
  status: "ready",
  transcriptVersion: 2,
  cloudUsed: false,
  consentConfirmed: false,
  job: null,
  ...over,
});

afterEach(cleanup);

describe("groupByDay", () => {
  it("groups by calendar day and keeps order", () => {
    const g = groupByDay([row("a", { startedAt: at(0, 14) }), row("b", { startedAt: at(0, 8) }), row("c", { startedAt: at(1) }), row("d", { startedAt: at(3) }), row("e", { startedAt: at(9) }), row("f", { startedAt: at(40) })], NOW);
    expect(g.map((x) => x.key.kind)).toEqual(["today", "yesterday", "weekday", "lastWeek", "month"]);
    expect(g[0]!.rows.map((r) => r.gid)).toEqual(["a", "b"]);
  });
});

describe("rowStatus", () => {
  it("maps core status to a pill", () => {
    expect(rowStatus(row("a"), undefined, false).status).toBe("ready");
    expect(rowStatus(row("a", { cloudUsed: true }), undefined, false).status).toBe("cloudEnhanced");
    expect(rowStatus(row("a"), undefined, true).status).toBe("needsNames");
    expect(rowStatus(row("a", { status: "failed" }), undefined, false).status).toBe("failed");
    expect(rowStatus(row("a", { status: "recording" }), undefined, false).status).toBe("recording");
    const job = { kind: "final_pass", progress: 0.4, waitingForModels: false };
    expect(rowStatus(row("a", { status: "processing", job }), undefined, false)).toEqual({ status: "processing", percent: 40 });
    expect(rowStatus(row("a", { status: "processing", job }), 0.75, false).percent).toBe(75);
  });
  it("waiting for models wins over processing", () => {
    const job = { kind: "final_pass", progress: null, waitingForModels: true };
    expect(rowStatus(row("a", { status: "processing", job }), undefined, false).status).toBe("waitingModels");
  });
});

describe("LibraryList", () => {
  it("renders day headings, rows and status text", () => {
    const job = { kind: "final_pass", progress: 0.62, waitingForModels: false };
    render(
      <PlatformProvider value="mac">
        <LibraryList
          now={NOW}
          onOpen={() => {}}
          rows={[row("a", { title: "Standup" }), row("b", { title: "Workshop", startedAt: at(1), status: "processing", job }), row("c", { title: "Queued", startedAt: at(1), status: "processing", job: { ...job, waitingForModels: true } })]}
        />
      </PlatformProvider>,
    );
    expect(screen.getByRole("heading", { name: "Today" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Yesterday" })).toBeTruthy();
    expect(screen.getByText("Processing 62%")).toBeTruthy();
    expect(screen.getByText("Waiting for models")).toBeTruthy();
    expect(screen.getAllByText(/2:05/).length).toBe(3);
  });

  it("asks before deleting", () => {
    const onDelete = vi.fn();
    render(
      <PlatformProvider value="mac">
        <LibraryList now={NOW} onOpen={() => {}} onDelete={onDelete} rows={[row("a", { title: "Standup" })]} />
      </PlatformProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(onDelete).not.toHaveBeenCalled();
    expect(screen.getByText(/Delete 1 meeting\?/)).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "Delete" }).at(-1)!);
    expect(onDelete).toHaveBeenCalledWith("a");
  });

  it("opens a row and keeps copy notes disabled", () => {
    const onOpen = vi.fn();
    render(
      <PlatformProvider value="mac">
        <LibraryList now={NOW} onOpen={onOpen} rows={[row("a", { title: "Standup" })]} />
      </PlatformProvider>,
    );
    const li = screen.getByRole("listitem");
    fireEvent.click(within(li).getByText("Standup"));
    expect(onOpen).toHaveBeenCalledWith("a");
    expect((within(li).getByRole("button", { name: "Copy notes" }) as HTMLButtonElement).disabled).toBe(true);
  });
});
