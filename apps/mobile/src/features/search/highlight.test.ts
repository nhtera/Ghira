// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { splitHighlights } from "./highlight";

describe("splitHighlights", () => {
  it("marks ranges on the original, accented text", () => {
    expect(
      splitHighlights("Đà Nẵng và đồng", [
        [0, 7],
        [11, 15],
      ]),
    ).toEqual([
      { text: "Đà Nẵng", hit: true },
      { text: " và ", hit: false },
      { text: "đồng", hit: true },
    ]);
  });

  it("returns the text as is without ranges", () => {
    expect(splitHighlights("abc", [])).toEqual([{ text: "abc", hit: false }]);
    expect(splitHighlights("", [])).toEqual([]);
  });

  it("clamps, sorts and skips overlapping or empty ranges", () => {
    expect(
      splitHighlights("abcdef", [
        [4, 99],
        [0, 2],
        [1, 3],
        [3, 3],
      ]),
    ).toEqual([
      { text: "ab", hit: true },
      { text: "c", hit: true },
      { text: "d", hit: false },
      { text: "ef", hit: true },
    ]);
  });
});
