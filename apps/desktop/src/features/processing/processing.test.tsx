// SPDX-License-Identifier: Apache-2.0
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { PlatformProvider } from "@ghi/ui";
import { ProcessingPanel } from "./processing-panel";
import { useProcessing } from "./processing-store";
import { stepsFor } from "./stepper-steps";

beforeEach(() => useProcessing.setState({ meetings: {}, finished: [] }));
afterEach(cleanup);

describe("processing store", () => {
  it("follows jobProgress stages and finishes on notesReady", () => {
    const { apply } = useProcessing.getState();
    apply({ type: "stateChanged", meeting: "m", state: "processing" });
    expect(useProcessing.getState().meetings.m).toMatchObject({ stage: null });
    apply({ type: "jobProgress", meeting: "m", job: 1, kind: "final_pass", stage: "matchingVoices", progress: 0.4 });
    expect(useProcessing.getState().meetings.m).toMatchObject({ stage: "matchingVoices", progress: 0.4 });
    apply({ type: "notesReady", meeting: "m", version: 2 });
    expect(useProcessing.getState().meetings.m).toBeUndefined();
    expect(useProcessing.getState().finished).toEqual(["m"]);
  });
  it("asks for names after v2 only, not after live notes (v1)", () => {
    const { apply } = useProcessing.getState();
    apply({ type: "notesReady", meeting: "m", version: 1 });
    expect(useProcessing.getState().finished).toEqual([]);
    apply({ type: "notesReady", meeting: "m", version: 2 });
    expect(useProcessing.getState().finished).toEqual(["m"]);
    useProcessing.getState().forget("m");
    expect(useProcessing.getState().finished).toEqual([]);
  });
  it("drops a meeting whose job failed (error event) or was deleted", () => {
    const { apply, forget } = useProcessing.getState();
    apply({ type: "jobProgress", meeting: "m", job: 1, kind: "final_pass", stage: "decoding", progress: 0 });
    apply({ type: "error", meeting: "m", kind: "job", message: "x" });
    expect(useProcessing.getState().meetings.m).toBeUndefined();
    apply({ type: "jobProgress", meeting: "n", job: 2, kind: "final_pass", stage: "decoding", progress: 0 });
    forget("n");
    expect(useProcessing.getState().meetings.n).toBeUndefined();
  });
  it("stays done after notesReady: the runner's done event and later jobs don't bring the stepper back", () => {
    const { apply } = useProcessing.getState();
    // The real order: final pass, notes, then the semantic index (job runner events included).
    apply({ type: "jobProgress", meeting: "m", job: 1, kind: "final_pass", stage: "improvingTranscript", progress: 0.9 });
    apply({ type: "jobProgress", meeting: "m", job: 1, kind: "final_pass", stage: null, progress: 1 });
    expect(useProcessing.getState().meetings.m).toMatchObject({ stage: "improvingTranscript" });
    apply({ type: "jobProgress", meeting: "m", job: 2, kind: "notes_final", stage: null, progress: 0 });
    apply({ type: "notesReady", meeting: "m", version: 2 });
    apply({ type: "jobProgress", meeting: "m", job: 2, kind: "notes_final", stage: null, progress: 1 });
    apply({ type: "jobProgress", meeting: "m", job: 3, kind: "embed_index", stage: null, progress: 0 });
    apply({ type: "jobProgress", meeting: "m", job: 3, kind: "embed_index", stage: null, progress: 0.5 });
    apply({ type: "jobProgress", meeting: "m", job: 4, kind: "voice_learn", stage: null, progress: 0 });
    expect(useProcessing.getState().meetings.m).toBeUndefined();
  });
  it("drops a meeting that failed or went idle", () => {
    const { apply } = useProcessing.getState();
    apply({ type: "jobProgress", meeting: "m", job: 1, kind: "final_pass", stage: "decoding", progress: 0 });
    apply({ type: "stateChanged", meeting: "m", state: "failed" });
    expect(useProcessing.getState().meetings.m).toBeUndefined();
  });
});

describe("stepsFor", () => {
  it("marks earlier stages done and the current one running", () => {
    expect(stepsFor({ stage: "improvingTranscript", kind: "final_pass" }).map((s) => s.status)).toEqual(["done", "done", "done", "running", "pending"]);
  });
  it("treats notes jobs without a stage as writing notes, and no stage as the first step", () => {
    expect(stepsFor({ stage: null, kind: "notes_final" })[4]!.status).toBe("running");
    expect(stepsFor({ stage: null, kind: "final_pass" })[0]!.status).toBe("running");
  });
});

describe("ProcessingPanel", () => {
  it("shows the stepper and the leave-the-page copy", () => {
    render(
      <PlatformProvider value="mac">
        <ProcessingPanel processing={{ stage: "refiningSpeakers", kind: "final_pass", progress: 0.2 }} />
      </PlatformProvider>,
    );
    expect(screen.getByText("Writing your notes on this Mac", { selector: "h2" })).toBeTruthy();
    expect(screen.getByText(/You can leave this page/)).toBeTruthy();
    expect(screen.getByRole("progressbar", { name: "Refining speakers" })).toBeTruthy();
    act(() => {});
  });
});
