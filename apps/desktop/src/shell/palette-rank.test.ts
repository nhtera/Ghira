// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { fold, rankPalette, type PaletteItem } from "./palette-rank";

const items: PaletteItem[] = [
  { id: "go-meetings", group: "goTo", label: "Meetings" },
  { id: "go-people", group: "goTo", label: "People" },
  { id: "rec-call", group: "actions", label: "Record call" },
  { id: "theme", group: "actions", label: "Switch to dark theme" },
  { id: "m1", group: "recent", label: "Product sync — beta scope" },
  { id: "m2", group: "recent", label: "Chốt lịch review với Linh" },
];
const ask = (q: string): PaletteItem => ({ id: "ask", group: "actions", label: `Ask “${q}”` });
const ids = (q: string) => rankPalette(q, items, ask).map((g) => [g.group, g.items.map((i) => i.id)]);

describe("command palette ranking", () => {
  it("folds case and Vietnamese accents", () => {
    expect(fold("Chốt lịch Đà Nẵng")).toBe("chot lich da nang");
  });

  it("empty query: Go to, Actions, Recent", () => {
    expect(ids("")).toEqual([
      ["goTo", ["go-meetings", "go-people"]],
      ["actions", ["rec-call", "theme"]],
      ["recent", ["m1", "m2"]],
    ]);
  });

  it("typing: Actions with Ask first, then Meetings, then Go to", () => {
    expect(ids("chot")).toEqual([
      ["actions", ["ask"]],
      ["meetings", ["m2"]],
    ]);
    expect(ids("re")).toEqual([
      ["actions", ["ask", "rec-call"]],
      ["meetings", ["m2"]],
    ]);
  });

  it("an unmatched query can still be asked", () => {
    expect(ids("pricing tiers")).toEqual([["actions", ["ask"]]]);
  });

  it("prefix matches rank first", () => {
    const r = rankPalette("p", items, ask);
    expect(r.find((g) => g.group === "goTo")?.items.map((i) => i.id)).toEqual(["go-people"]);
    expect(r.find((g) => g.group === "meetings")?.items.map((i) => i.id)).toEqual(["m1"]);
  });
});
