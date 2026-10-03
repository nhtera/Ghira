// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider } from "@ghi/ui";
import type { MeetingRow } from "../../bindings";
import { MeetingRowView } from "./meeting-row";

afterEach(cleanup);

const row: MeetingRow = {
  gid: "a",
  title: "Standup",
  startedAt: Date.UTC(2026, 9, 1, 9, 30),
  durationMs: 12 * 60_000,
  source: "live",
  mode: "call",
  status: "failed",
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
};
const view = (over: Partial<MeetingRow>, status: Parameters<typeof MeetingRowView>[0]["status"], onRetry = vi.fn()) =>
  render(
    <PlatformProvider value="mac">
      <MeetingRowView row={{ ...row, ...over }} status={status} title="Standup" locale="en" onOpen={() => {}} onRetry={onRetry} actions={<button>Open</button>} />
    </PlatformProvider>,
  );

describe("MeetingRowView", () => {
  it("a failed row's Retry pill stays clickable and the hover actions move aside", () => {
    const onRetry = vi.fn();
    const { container } = view({}, { status: "failed" }, onRetry);
    fireEvent.click(screen.getByRole("button", { name: "Failed · Retry" }));
    expect(onRetry).toHaveBeenCalledOnce();
    expect(container.querySelector(".shadow-float")?.className).toContain("right-[226px]");
  });
  it("names the source for screen readers and shows the length", () => {
    view({ status: "ready" }, { status: "ready" });
    expect(screen.getByText("Call", { selector: ".sr-only" })).toBeTruthy();
    expect(screen.getByText("12 min")).toBeTruthy();
  });
  it("shows no length while processing or when it is unknown, and no pill without a status", () => {
    const { container, rerender } = view({ status: "processing" }, { status: "processing", percent: 10 });
    expect(screen.queryByText(/min/)).toBeNull();
    rerender(
      <PlatformProvider value="mac">
        <MeetingRowView row={{ ...row, status: "ready", durationMs: 0 }} status={null} title="Standup" locale="en" onOpen={() => {}} />
      </PlatformProvider>,
    );
    expect(screen.queryByText(/min/)).toBeNull();
    expect(container.querySelector("[data-status]")).toBeNull();
  });
});
