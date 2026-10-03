// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { i18next } from "@ghi/i18n";
import { durationLabel } from "./duration";

describe("durationLabel", () => {
  const t = i18next.t.bind(i18next);
  it("minutes, then hours and minutes", () => {
    expect(durationLabel(t, 28 * 60_000)).toBe("28 min");
    expect(durationLabel(t, 94 * 60_000)).toBe("1 h 34 min");
    expect(durationLabel(t, 120 * 60_000)).toBe("2 h");
    expect(durationLabel(t, 5_000)).toBe("1 min");
  });
});
