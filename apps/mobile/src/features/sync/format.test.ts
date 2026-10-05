// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { relativeTime } from "./format";

const NOW = 1_800_000_000_000;

describe("relativeTime", () => {
  it("counts minutes, hours and days back", () => {
    expect(relativeTime(NOW - 4 * 60_000, NOW, "en")).toBe("4 minutes ago");
    expect(relativeTime(NOW - 3 * 3_600_000, NOW, "en")).toBe("3 hours ago");
    expect(relativeTime(NOW - 2 * 86_400_000, NOW, "en")).toBe("2 days ago");
  });

  it("says now for less than a minute", () => {
    expect(relativeTime(NOW - 10_000, NOW, "en")).toBe("this minute");
  });

  it("speaks Vietnamese", () => {
    expect(relativeTime(NOW - 4 * 60_000, NOW, "vi")).toBe("4 phút trước");
  });
});
