// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { StatusPill, type StatusKind } from "./status-pill";

const LABELS: Record<StatusKind, string> = {
  ready: "Ready",
  processing: "Processing 62%",
  needsNames: "Needs speaker names",
  cloudEnhanced: "Cloud-enhanced",
  failed: "Failed · Retry",
  waitingSync: "Waiting for sync",
  waitingModels: "Waiting for models",
  recording: "Recording",
};

afterEach(cleanup);

describe("StatusPill", () => {
  it.each(Object.entries(LABELS))("%s shows an icon and text", (status, text) => {
    const { container } = render(<StatusPill status={status as StatusKind} percent={62} />);
    expect(screen.getByText(text)).toBeTruthy();
    expect(container.querySelector("svg[data-icon]")).toBeTruthy();
  });

  it("processing without a percent has no number", () => {
    render(<StatusPill status="processing" />);
    expect(screen.getByText("Processing")).toBeTruthy();
  });

  it("failed is a retry button only when onRetry is given", async () => {
    const onRetry = vi.fn();
    const { rerender } = render(<StatusPill status="failed" />);
    expect(screen.queryByRole("button")).toBeNull();
    rerender(<StatusPill status="failed" onRetry={onRetry} />);
    await userEvent.click(screen.getByRole("button", { name: "Failed · Retry" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });
});
