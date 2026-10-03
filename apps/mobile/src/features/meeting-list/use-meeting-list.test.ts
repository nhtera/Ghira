// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { chipWithProgress } from "./use-meeting-list";

describe("chipWithProgress", () => {
  it("leaves the chip alone without progress", () => {
    expect(chipWithProgress({ kind: "synced" }, undefined)).toEqual({
      kind: "synced",
    });
    expect(chipWithProgress(undefined, undefined)).toBeUndefined();
  });

  it("shows the live percent while the phone processes", () => {
    expect(
      chipWithProgress({ kind: "processingOnPhone", percent: 10 }, 70),
    ).toEqual({ kind: "processingOnPhone", percent: 70 });
    expect(chipWithProgress({ kind: "recorded" }, 5)).toEqual({
      kind: "processingOnPhone",
      percent: 5,
    });
    expect(chipWithProgress(undefined, 5)).toEqual({
      kind: "processingOnPhone",
      percent: 5,
    });
  });

  it("does not turn a finished meeting back into processing", () => {
    expect(chipWithProgress({ kind: "processedOnPhone" }, 50)).toEqual({
      kind: "processedOnPhone",
    });
    expect(chipWithProgress({ kind: "failed" }, 50)).toEqual({
      kind: "failed",
    });
  });
});
