// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { isSyncAvailable, isSyncPaired, MOCK_SYNC_MEETING, createSyncMock } from "./mock-sync";

describe("the sync mock's meeting and target overlays", () => {
  it("dresses one meeting for a final pass open on the computer, and revoking ends it", async () => {
    const m = createSyncMock();
    m.hooks.syncSet("leased");
    const chips = await m.commands.meetingChips([MOCK_SYNC_MEETING, "m-fail"]);
    expect(chips.status === "ok" && chips.data.find((c) => c.gid === MOCK_SYNC_MEETING)?.chip).toEqual({ kind: "finalOnDesktop", percent: 42 });
    const detail = await m.commands.meetingDetail(MOCK_SYNC_MEETING);
    expect(detail.status === "ok" && detail.data.audioAvailable).toBe(false);
    await m.commands.syncLeaseRevoke(MOCK_SYNC_MEETING);
    const after = await m.commands.meetingChips([MOCK_SYNC_MEETING]);
    expect(after.status === "ok" && after.data[0]?.chip).toEqual({ kind: "synced" });
    expect(m.revoked).toEqual([MOCK_SYNC_MEETING]);
  });

  it("offers pairing after any state is set, and a Desktop default only while paired", async () => {
    const m = createSyncMock();
    expect(isSyncAvailable()).toBe(false);
    expect(isSyncPaired()).toBe(false);
    const refused = await m.commands.setMobileSettings({ defaultTarget: "desktop", modelsWifiOnly: true } as never);
    expect(refused).toEqual({ status: "error", error: "pairingNotAvailable" });
    m.hooks.syncSet("paired");
    expect(isSyncAvailable()).toBe(true);
    expect(isSyncPaired()).toBe(true);
    const ok = await m.commands.setMobileSettings({ defaultTarget: "desktop", modelsWifiOnly: true } as never);
    expect(ok.status === "ok" && ok.data.defaultTarget).toBe("desktop");
    m.hooks.syncSet("off");
    expect(isSyncPaired()).toBe(false);
  });

  it("fails the next scan start with the given code, once", async () => {
    const m = createSyncMock();
    m.hooks.syncSet("pairing");
    m.hooks.syncFailNextScan("expired");
    expect(await m.commands.syncPairScanStart()).toEqual({ status: "error", error: "expired" });
    expect(await m.commands.syncPairScanStart()).toEqual({ status: "ok", data: null });
  });
});
