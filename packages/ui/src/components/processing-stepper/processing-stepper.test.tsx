// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ProcessingStepper, type ProcessingStep } from "./processing-stepper";

const steps = (over: Partial<ProcessingStep>[]): ProcessingStep[] =>
  (["refiningSpeakers", "matchingVoices", "improvingTranscript", "writingNotes"] as const).map((id, i) => ({
    id,
    status: "pending",
    ...over[i],
  }));



afterEach(cleanup);

describe("ProcessingStepper", () => {
  it("is an ordered list with the four stage names", () => {
    render(<ProcessingStepper steps={steps([])} />);
    const items = within(screen.getByRole("list", { name: "Steps to write your notes" })).getAllByRole("listitem");
    expect(items.map((i) => within(i).getByText(/Refining|Matching|Improving|Writing/).textContent)).toEqual([
      "Refining speakers",
      "Matching voices",
      "Improving transcript",
      "Writing notes",
    ]);
  });

  it("running step marks aria-current and exposes progress and estimate", () => {
    render(<ProcessingStepper steps={steps([{ status: "done" }, { status: "skipped" }, { status: "running", progress: 45, estimateSeconds: 40 }])} />);
    const bar = screen.getByRole("progressbar", { name: "Improving transcript" });
    expect(bar.getAttribute("aria-valuenow")).toBe("45");
    expect(bar.closest("li")?.getAttribute("aria-current")).toBe("step");
    expect(screen.getByText("~40 s")).toBeTruthy();
    expect(screen.getAllByRole("progressbar")).toHaveLength(1);
  });

  it("estimates over 90 s are minutes; unknown progress draws no bar", () => {
    render(<ProcessingStepper steps={steps([{ status: "running", estimateSeconds: 210 }, { status: "pending", estimateSeconds: 40 }])} />);
    expect(screen.getByText("~4 min")).toBeTruthy();
    expect(screen.getByText("~40 s")).toBeTruthy();
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("shows each status as text", () => {
    render(<ProcessingStepper steps={steps([{ status: "done" }, { status: "skipped" }, { status: "failed" }, { status: "pending" }])} />);
    for (const t of ["Done", "Skipped", "Failed", "Pending"]) expect(screen.getByText(t)).toBeTruthy();
  });

  it("failed step retries by stage", async () => {
    const onRetry = vi.fn();
    render(<ProcessingStepper steps={steps([{ status: "done" }, { status: "done" }, { status: "done" }, { status: "failed" }])} onRetry={onRetry} />);
    await userEvent.click(screen.getByRole("button", { name: "Try again: Writing notes" }));
    expect(onRetry).toHaveBeenCalledWith("writingNotes");
  });

  it("supports the decoding stage before the four final-pass stages", () => {
    render(<ProcessingStepper steps={[{ id: "decoding", status: "running", progress: 70 }, ...steps([])]} />);
    expect(screen.getByRole("progressbar", { name: "Reading the recording" }).getAttribute("aria-valuenow")).toBe("70");
  });
});
