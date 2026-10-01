// SPDX-License-Identifier: Apache-2.0
import { expect, test } from "vitest";
import { formatBytes, formatClock, formatDate, formatTime } from "../src/format";

test("dates and times per locale (brief §8)", () => {
  const d = new Date(2026, 8, 28, 14, 5);
  expect(formatDate(d, "vi")).toBe("28/09/2026");
  expect(formatDate(d, "en")).toBe("Sep 28, 2026");
  expect(formatTime(d, "vi")).toBe("14:05");
  expect(formatTime(d, "en")).toMatch(/^2:05\s?PM$/);
});

test("meeting clock", () => {
  expect(formatClock(9_000)).toBe("0:09");
  expect(formatClock(2_527_000)).toBe("42:07");
  expect(formatClock(3_727_000)).toBe("1:02:07");
});

test("sizes use the locale's decimal separator", () => {
  expect(formatBytes(1.2e9, "en")).toBe("1.2 GB");
  expect(formatBytes(1.2e9, "vi")).toBe("1,2 GB");
  expect(formatBytes(34e9, "en")).toBe("34 GB");
  expect(formatBytes(340e6, "en")).toBe("340 MB");
  expect(formatBytes(null, "en")).toBe("");
});
