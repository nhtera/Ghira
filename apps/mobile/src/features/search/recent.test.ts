// SPDX-License-Identifier: Apache-2.0
import { beforeEach, describe, expect, it } from "vitest";
import {
  clearRecentSearches,
  loadRecent,
  MAX_RECENT,
  RECENT_CLEARED,
  saveRecent,
  withRecent,
} from "./recent";

describe("recent searches", () => {
  beforeEach(() => localStorage.clear());

  it("puts the newest first without repeats", () => {
    expect(withRecent(["a", "B"], "b")).toEqual(["b", "a"]);
    expect(withRecent(["a"], "  ")).toEqual(["a"]);
  });

  it("keeps a bounded list", () => {
    const many = Array.from({ length: MAX_RECENT }, (_, i) => `q${i}`);
    expect(withRecent(many, "new")).toHaveLength(MAX_RECENT);
  });

  it("round-trips through localStorage and survives bad data", () => {
    saveRecent(["đồng", "da nang"]);
    expect(loadRecent()).toEqual(["đồng", "da nang"]);
    localStorage.setItem("ghi.search.recent", "{oops");
    expect(loadRecent()).toEqual([]);
    saveRecent([]);
    expect(localStorage.getItem("ghi.search.recent")).toBeNull();
  });

  it("clearRecentSearches wipes the list and tells open screens", () => {
    saveRecent(["a"]);
    let told = 0;
    window.addEventListener(RECENT_CLEARED, () => told++, { once: true });
    clearRecentSearches();
    expect(loadRecent()).toEqual([]);
    expect(told).toBe(1);
  });
});
