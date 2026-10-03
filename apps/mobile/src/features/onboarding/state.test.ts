// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { isTransient, resumeStep, STEPS } from "./state";

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

  it("ends on done, and ignores the pair step", () => {
    expect(resumeStep({ completed: [...STEPS] })).toBe("done");
    expect(
      resumeStep({
        completed: ["languages", "micPriming", "consent", "pair"],
      }),
    ).toBe("processing");
    expect(STEPS).not.toContain("pair");
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
