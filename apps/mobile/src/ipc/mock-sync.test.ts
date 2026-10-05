// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { createSyncMock } from "./mock-sync";

const data = async <T>(p: Promise<{ status: string; data?: T }>) => (await p).data as T;

describe("the sync mock", () => {
  it("starts off, local only, nobody paired", async () => {
    const m = createSyncMock();
    const s = await data<{ enabled: boolean; localOnly: boolean; paired: unknown[] }>(m.commands.syncStatus() as never);
    expect(s).toMatchObject({ enabled: false, localOnly: true, paired: [] });
  });

  it("walks pairing, a conflict and an unpair", async () => {
    const m = createSyncMock();
    const seen: string[] = [];
    m.listeners.add((e) => seen.push(e.type));
    await m.commands.syncSetEnabled(true);
    m.hooks.syncSet("conflict");
    const copies = await data<{ gid: string }[]>(m.commands.syncConflicts("m1") as never);
    expect(copies).toHaveLength(1);
    await m.commands.syncConflictResolve(copies[0].gid, true);
    expect(await data<unknown[]>(m.commands.syncConflicts("m1") as never)).toHaveLength(0);
    const [d] = await data<{ gid: string }[]>(m.commands.syncDevices() as never);
    await m.commands.syncUnpair(d.gid);
    expect(seen).toEqual(["conflict", "unpaired"]);
    expect(await data<unknown[]>(m.commands.syncDevices() as never)).toEqual([]);
  });

  it("keeps a wiped device pending until the wipe is done", async () => {
    const m = createSyncMock();
    m.hooks.syncSet("paired");
    const [d] = await data<{ gid: string }[]>(m.commands.syncDevices() as never);
    await m.commands.syncUnpairAndWipe(d.gid);
    expect((await data<{ state: string }[]>(m.commands.syncDevices() as never))[0].state).toBe("wipePending");
    m.hooks.syncSimulateWipeDone();
    expect(await data<unknown[]>(m.commands.syncDevices() as never)).toEqual([]);
  });
});

describe("the phone scan", () => {
  it("pairs with the computer when a scan arrives", async () => {
    const m = createSyncMock();
    await m.commands.syncSetEnabled(true);
    await m.commands.syncPairScanStart();
    m.hooks.syncSimulateScan();
    const [d] = await data<{ platform: string }[]>(m.commands.syncDevices() as never);
    expect(d.platform).toBe("mac");
  });
});
