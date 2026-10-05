// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { errorKey, formatCountdown, qrDataUrl, whenAgo } from "./logic";

describe("sync logic", () => {
  it("counts down m:ss, rounding up", () => {
    expect(formatCountdown(120_000)).toBe("2:00");
    expect(formatCountdown(61_001)).toBe("1:02");
    expect(formatCountdown(500)).toBe("0:01");
    expect(formatCountdown(-5)).toBe("0:00");
  });

  it("words a past time in the app language", () => {
    const now = 10_000_000_000;
    expect(whenAgo(now - 20_000, now, "en", "just now")).toBe("just now");
    expect(whenAgo(now - 4 * 60_000, now, "en", "just now")).toMatch(/^4 min/);
    expect(whenAgo(now - 4 * 60_000, now, "vi", "vừa xong")).toMatch(/4 phút/);
    expect(whenAgo(now - 3 * 3_600_000, now, "en", "x")).toMatch(/3 hr/);
    expect(whenAgo(now - 2 * 86_400_000, now, "en", "x")).toMatch(/2 days/);
  });

  it("only turns an SVG document into an image source", () => {
    expect(qrDataUrl('<svg xmlns="http://www.w3.org/2000/svg"/>')).toMatch(/^data:image\/svg\+xml/);
    expect(qrDataUrl("<html></html>")).toBeNull();
    expect(qrDataUrl("")).toBeNull();
  });

  it("words an unknown error code as internal", () => {
    expect(errorKey("unreachable")).toBe("unreachable");
    expect(errorKey("weird")).toBe("internal");
  });
});
