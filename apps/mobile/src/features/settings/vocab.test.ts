// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { addTerm } from "./vocab";

describe("addTerm", () => {
  it("trims and collapses spaces", () => {
    expect(addTerm([], "  Nguyễn   Văn An ", 200)).toEqual({ terms: ["Nguyễn Văn An"], status: "added" });
  });
  it("ignores an empty entry", () => {
    expect(addTerm(["a"], "   ", 200)).toEqual({ terms: ["a"], status: "empty" });
  });
  it("refuses a duplicate without case or accents", () => {
    expect(addTerm(["Chốt"], "chot", 200).status).toBe("duplicate");
    expect(addTerm(["Đạt"], "DAT", 200).status).toBe("duplicate");
  });
  it("refuses a term past the cap", () => {
    const full = Array.from({ length: 3 }, (_, i) => `t${i}`);
    expect(addTerm(full, "new", 3)).toEqual({ terms: full, status: "full" });
    expect(addTerm(full, "new", 4).status).toBe("added");
  });
});
