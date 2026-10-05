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
    const hidden = hiddenAfter(1, 2, true);
    expect(hidden).toBe(1);
    expect(bannerVisible(2, hidden)).toBe(true);
    expect(hiddenAfter(5, null, true)).toBeNull();
  });
  it("keeps it closed across a lock and unlock with the same files", () => {
    let hidden: number | null = 2;
    // Locked: the list is cleared (0 waiting) but not loaded.
    hidden = hiddenAfter(0, hidden, false);
    expect(hidden).toBe(2);
    // Unlocked, list reloaded: the same two files.
    hidden = hiddenAfter(2, hidden, true);
    expect(bannerVisible(2, hidden)).toBe(false);
    // And a third still brings it back.
    expect(bannerVisible(3, hidden)).toBe(true);
  });
});
