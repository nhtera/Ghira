// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { ALL_STEPS, flowSteps, nextStep, prevStep, resolveStep } from "./steps";

describe("step machine", () => {
  it("skips the voice step while voice profiles are off", () => {
    expect(flowSteps(false)).toEqual(["welcome", "languages", "models", "permissions", "test", "recovery", "done"]);
    expect(flowSteps(true)).toEqual([...ALL_STEPS]);
  });

  it("moves next and back through the flow without the voice step", () => {
    expect(nextStep("permissions", false)).toBe("test");
    expect(prevStep("test", false)).toBe("permissions");
    expect(nextStep("permissions", true)).toBe("voice");
    expect(prevStep("test", true)).toBe("voice");
  });

  it("stays at both ends", () => {
    expect(prevStep("welcome", false)).toBe("welcome");
    expect(nextStep("done", false)).toBe("done");
  });

  it("walks a hidden step to its neighbour, and a bad URL to the start", () => {
    expect(resolveStep("voice", false)).toBe("test");
    expect(resolveStep("voice", true)).toBe("voice");
    expect(nextStep("voice", false)).toBe("test");
    expect(prevStep("voice", false)).toBe("permissions");
    expect(resolveStep("nope", false)).toBe("welcome");
    expect(resolveStep(undefined, false)).toBe("welcome");
  });
});
