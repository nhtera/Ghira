// SPDX-License-Identifier: Apache-2.0
import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  document.documentElement.lang = "en";
});

// The sample's due words are stored per language: an item shows one of them, never both glued together.
describe("mock action item due text", () => {
  const dues = async (lang: "en" | "vi") => {
    // The mock builds its sample once, in the language of the page at that moment.
    vi.resetModules();
    document.documentElement.lang = lang;
    const { mockIpc } = await import("./mock");
    const rows = await mockIpc.commands.listMeetings(50, 0);
    if (rows.status !== "ok") throw new Error("no meetings");
    const id = rows.data.find((r) => r.status === "ready")!.gid;
    const notes = await mockIpc.commands.meetingNotes(id);
    if (notes.status !== "ok") throw new Error("no notes");
    return notes.data.actionItems.map((a) => a.dueText);
  };

  it("English", async () => {
    const d = await dues("en");
    expect(d).toContain("~2 weeks");
    expect(d.join("|")).not.toMatch(/tuần|Thứ/);
  });
  it("Vietnamese", async () => {
    const d = await dues("vi");
    expect(d).toContain("~2 tuần");
    expect(d.join("|")).not.toMatch(/weeks|Fri/);
  });
  it("an empty due is no due", async () => {
    expect((await dues("en")).every((x) => x === null || x !== "")).toBe(true);
  });
});
