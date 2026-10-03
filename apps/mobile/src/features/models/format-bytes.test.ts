// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { formatBytes } from "./format-bytes";

describe("formatBytes", () => {
  it("uses MB below 1 GB and GB above", () => {
    expect(formatBytes(340e6, "en")).toMatch(/340\s?MB/);
    expect(formatBytes(1.24e9, "en")).toMatch(/1\.2\s?GB/);
  });
  it("is empty when the size is unknown", () => {
    expect(formatBytes(null, "en")).toBe("");
  });
});
