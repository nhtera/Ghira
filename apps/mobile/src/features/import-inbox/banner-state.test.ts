// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { bannerVisible, hiddenAfter } from "./banner-state";

describe("the waiting banner", () => {
  it("shows while files wait and it was not closed", () => {
    expect(bannerVisible(2, null)).toBe(true);
    expect(bannerVisible(0, null)).toBe(false);
  });
  it("stays closed at the same or a lower count, returns when one more arrives", () => {
    expect(bannerVisible(2, 2)).toBe(false);
    expect(bannerVisible(1, 2)).toBe(false);
    expect(bannerVisible(3, 2)).toBe(true);
  });
  it("a file arriving after some were handled counts as new", () => {
    const hidden = hiddenAfter(1, 2);
    expect(hidden).toBe(1);
    expect(bannerVisible(2, hidden)).toBe(true);
    expect(hiddenAfter(5, null)).toBeNull();
  });
});
