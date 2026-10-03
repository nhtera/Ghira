// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { fold, isDeletePhrase } from "./fold";

describe("delete phrase", () => {
  it("ignores case and accents in both languages", () => {
    for (const ok of [
      "DELETE",
      "delete",
      " Delete ",
      "XÓA",
      "xóa",
      "xoa",
      "XOA",
    ])
      expect(isDeletePhrase(ok), ok).toBe(true);
  });
  it("refuses anything else", () => {
    for (const bad of ["", "del", "deleted", "xoá hết", "yes"])
      expect(isDeletePhrase(bad), bad).toBe(false);
  });
  it("folds đ like d", () => {
    expect(fold("Đà Nẵng")).toBe("da nang");
  });
});
