// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { isTransient, resumeStep, STEPS, stepsFor } from "./state";

describe("resumeStep", () => {
  it("starts at the first step", () => {
    expect(resumeStep({ completed: [] })).toBe("languages");
  });

  it("resumes at the first step not completed, in order, whatever the order completed in", () => {
    expect(resumeStep({ completed: ["micPriming", "languages"] })).toBe(
      "consent",
    );
    expect(resumeStep({ completed: ["languages", "voice"] })).toBe(
      "micPriming",
    );
  });

  it("ends on done, and ignores the pair step while sync is unavailable", () => {
    expect(resumeStep({ completed: [...STEPS] })).toBe("done");
    expect(
      resumeStep({
        completed: ["languages", "micPriming", "consent", "pair"],
      }),
    ).toBe("processing");
    expect(STEPS).not.toContain("pair");
  });
});

describe("stepsFor", () => {
  it("is the plain list without sync", () => {
    expect(stepsFor(false)).toEqual(STEPS);
  });

  it("puts pair right before processing with sync", () => {
    const steps = stepsFor(true);
    expect(steps.indexOf("pair")).toBe(steps.indexOf("processing") - 1);
    expect(steps.indexOf("pair")).toBe(steps.indexOf("consent") + 1);
    expect(steps.filter((s) => s !== "pair")).toEqual(STEPS);
  });

  it("resumes at pair when it is the first step missing", () => {
    const completed = ["languages", "micPriming", "consent"] as const;
    expect(resumeStep({ completed: [...completed], syncAvailable: true })).toBe("pair");
    expect(resumeStep({ completed: [...completed], syncAvailable: false })).toBe("processing");
    expect(resumeStep({ completed: [...completed, "pair"], syncAvailable: true })).toBe("processing");
  });
});

describe("isTransient", () => {
  it("tells a locked or starting store from a failure", () => {
    expect(isTransient("locked")).toBe(true);
    expect(isTransient("starting")).toBe(true);
    expect(isTransient("disk I/O error")).toBe(false);
  });

  it("puts the optional voice step after the models", () => {
    expect(STEPS.indexOf("voice")).toBeGreaterThan(STEPS.indexOf("models"));
  });
});
