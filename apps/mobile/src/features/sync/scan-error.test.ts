// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { needsSettings, scanFailure, scanFailureKey, syncErrorKey, wantsHotspot } from "./scan-error";

describe("scanFailure", () => {
  it("keeps the codes the scan itself reports", () => {
    for (const c of ["cameraOff", "invalid", "expired", "notFound", "localNetwork"]) expect(scanFailure(c)).toBe(c);
  });

  it("reads the sync error codes as scan failures", () => {
    expect(scanFailure("unreachable")).toBe("notFound");
    expect(scanFailure("refused")).toBe("expired");
    expect(scanFailure("storageFull")).toBe("storageFull");
  });

  it("treats anything else as not a code", () => {
    expect(scanFailure("who knows")).toBe("invalid");
  });
});

describe("what each failure offers", () => {
  it("points the permission failures to Settings and only a missing computer to Personal Hotspot", () => {
    expect(needsSettings("cameraOff")).toBe(true);
    expect(needsSettings("localNetwork")).toBe(true);
    expect(needsSettings("notFound")).toBe(false);
    expect(wantsHotspot("notFound")).toBe(true);
    expect(wantsHotspot("expired")).toBe(false);
    expect(wantsHotspot("localNetwork")).toBe(false);
  });

  it("words scan failures under scan.* and the rest under error.*", () => {
    expect(scanFailureKey("invalid")).toBe("scan.invalid");
    expect(scanFailureKey("locked")).toBe("error.locked");
  });

  it("words a failed session, unknown codes as internal", () => {
    expect(syncErrorKey("unreachable")).toBe("unreachable");
    expect(syncErrorKey("upgradeRequired")).toBe("upgradeRequired");
    expect(syncErrorKey("anything")).toBe("internal");
  });
});
