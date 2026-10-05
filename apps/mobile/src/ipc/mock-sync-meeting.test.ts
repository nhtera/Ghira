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
    // The core's own fields: the audio lives on the computer, and its final pass is open there.
    expect(detail.status === "ok" && detail.data.audioOnPeer).toBe(true);
    expect(detail.status === "ok" && detail.data.leaseOpen).toEqual({ device: "MacBook Pro", percent: 42 });
    await m.commands.syncLeaseRevoke(MOCK_SYNC_MEETING);
    const taken = await m.commands.meetingDetail(MOCK_SYNC_MEETING);
    expect(taken.status === "ok" && taken.data.leaseOpen).toBeUndefined();
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

  it("keeps a meeting's audio here unless the computer has it", async () => {
    const m = createSyncMock();
    m.hooks.syncSet("paired");
    const detail = await m.commands.meetingDetail(MOCK_SYNC_MEETING);
    expect(detail.status === "ok" && detail.data.audioOnPeer).toBeFalsy();
    expect(detail.status === "ok" && detail.data.leaseOpen).toBeUndefined();
  });

  it("accepts the Desktop target for imports and keeps the offline hours in range", async () => {
    const m = createSyncMock();
    m.hooks.syncSet("paired");
    const saved = await m.commands.setMobileSettings({ defaultTarget: "desktop", modelsWifiOnly: true, desktopOfflineHours: 100_000 });
    expect(saved.status === "ok" && saved.data).toMatchObject({ defaultTarget: "desktop", desktopOfflineHours: 168 });
  });

  it("delete everything waits for the computer until it answers or the wait is skipped", async () => {
    const m = createSyncMock();
    m.hooks.syncSet("paired");
    const done = m.commands.privacyDeleteAll("DELETE", true);
    await Promise.resolve();
    expect(await m.commands.syncDeleteEverywhereStatus()).toEqual({ status: "ok", data: { state: "waiting", waitingFor: ["MacBook Pro"] } });
    await m.commands.syncDeleteEverywhereSkip();
    expect(await done).toEqual({ status: "ok", data: null });
    expect(await m.commands.syncDeleteEverywhereStatus()).toEqual({ status: "ok", data: { state: "done", waitingFor: [] } });
    // A wrong phrase never waits.
    expect(await m.commands.privacyDeleteAll("nope", true)).toEqual({ status: "error", error: "confirmation" });
  });
});
