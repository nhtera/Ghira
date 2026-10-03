// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { classifyError } from "./read-error";

describe("classifyError", () => {
  it("tells locked, starting and missing apart from other failures", () => {
    expect(classifyError("the app is locked")).toBe("locked");
    expect(classifyError("the app is starting")).toBe("starting");
    expect(classifyError("meeting not found")).toBe("missing");
    expect(classifyError("all data is being deleted")).toBe("failed");
    expect(classifyError("disk I/O error")).toBe("failed");
  });
});
