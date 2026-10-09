// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { openQuestionStarters } from "./starters";

const q = (text: string, kind = "question") => ({ kind, text });

describe("openQuestionStarters", () => {
  it("keeps real questions only: ASCII and full-width question marks, English and Vietnamese", () => {
    const r = openQuestionStarters([q("Who owns it?"), q("Ai phụ trách？"), q("Needs a date"), q("Is it a decision?", "decision"), q("Câu hỏi?  ")]);
    expect(r).toEqual(["Who owns it?", "Ai phụ trách？", "Câu hỏi?"]);
  });

  it("is at most three, distinct, in the notes' order, with spaces tidied", () => {
    const r = openQuestionStarters([q("A?"), q("a?"), q("B \n  b?"), q("C?"), q("D?")]);
    expect(r).toEqual(["A?", "B b?", "C?"]);
  });

  it("is empty without questions", () => {
    expect(openQuestionStarters([])).toEqual([]);
    expect(openQuestionStarters([q("no mark")])).toEqual([]);
  });
});
