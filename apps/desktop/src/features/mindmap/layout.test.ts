// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { COL_GAP, NODE_MAX_W, measureNode, tidyLayout, wrapLines, type Layout } from "./layout";
import type { MapNode } from "./tree";

/** Small deterministic PRNG (mulberry32), so a failing tree can be replayed by its seed. */
function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const WORDS = ["kế", "hoạch", "release", "đổi", "tên", "người", "nói", "voice", "profile", "consent", "a", "meeting", "quyết định", "supercalifragilisticexpialidocious"];

function randomTree(seed: number, leaves: number): MapNode {
  const r = rng(seed);
  const text = () => Array.from({ length: 1 + Math.floor(r() * 14) }, () => WORDS[Math.floor(r() * WORDS.length)]).join(" ");
  let n = 0;
  const node = (kind: MapNode["kind"], t: string): MapNode => ({ id: `n${n++}`, kind, text: t, full: t, children: [] });
  const root = node("root", "Title");
  let left = leaves;
  while (left > 0) {
    const sec = node("section", text());
    const k = Math.min(left, 1 + Math.floor(r() * 12));
    for (let i = 0; i < k; i++) {
      const l = node("leaf", text());
      if (r() < 0.2) l.proposed = true;
      if (r() < 0.2) l.ownerGid = "s1";
      // a few deeper levels
      if (r() < 0.15) l.children.push(node("leaf", text()), node("leaf", text()));
      sec.children.push(l);
    }
    left -= k;
    root.children.push(sec);
  }
  return root;
}

const overlaps = (l: Layout) => {
  const bad: string[] = [];
  for (let i = 0; i < l.nodes.length; i++)
    for (let j = i + 1; j < l.nodes.length; j++) {
      const a = l.nodes[i]!;
      const b = l.nodes[j]!;
      if (a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h) bad.push(`${a.id}/${b.id}`);
    }
  return bad;
};

describe("tidyLayout", () => {
  it("places 200-leaf random trees without any overlap, inside the reported size", () => {
    for (let seed = 1; seed <= 25; seed++) {
      const l = tidyLayout(randomTree(seed, 200));
      expect(overlaps(l), `seed ${seed}`).toEqual([]);
      for (const n of l.nodes) {
        expect(n.x).toBeGreaterThanOrEqual(0);
        expect(n.y).toBeGreaterThanOrEqual(0);
        expect(n.x + n.w).toBeLessThanOrEqual(l.width + 1e-6);
        expect(n.y + n.h).toBeLessThanOrEqual(l.height + 1e-6);
      }
    }
  });

  it("is deterministic", () => {
    const a = tidyLayout(randomTree(7, 200));
    const b = tidyLayout(randomTree(7, 200));
    expect(JSON.stringify(a.nodes.map((n) => [n.id, n.x, n.y, n.w, n.h]))).toBe(JSON.stringify(b.nodes.map((n) => [n.id, n.x, n.y, n.w, n.h])));
    expect(a.links).toEqual(b.links);
  });

  it("puts each depth in its own column, parents left of children, parent centered on its children", () => {
    const root = randomTree(3, 40);
    const l = tidyLayout(root);
    const by = new Map(l.nodes.map((n) => [n.id, n]));
    for (const { from, to } of l.links) expect(by.get(from)!.x + by.get(from)!.w + COL_GAP).toBeLessThanOrEqual(by.get(to)!.x + 1e-6);
    const col = new Map<number, number>();
    for (const n of l.nodes) {
      expect(col.get(n.depth) ?? n.x).toBe(n.x);
      col.set(n.depth, n.x);
    }
    // The root sits level with the middle of its sections.
    const kids = root.children.map((c) => by.get(c.id)!);
    const mid = (kids[0]!.y + kids[0]!.h / 2 + kids.at(-1)!.y + kids.at(-1)!.h / 2) / 2;
    const r = by.get("n0")!;
    expect(Math.abs(r.y + r.h / 2 - mid)).toBeLessThan(40);
  });

  it("leaves out the children of collapsed nodes, and a collapsed tree is shorter", () => {
    const root = randomTree(5, 60);
    const full = tidyLayout(root);
    const closed = tidyLayout(root, new Set(root.children.map((c) => c.id)));
    expect(closed.nodes).toHaveLength(1 + root.children.length);
    expect(closed.height).toBeLessThan(full.height);
    expect(overlaps(closed)).toEqual([]);
  });

  it("handles a lone root", () => {
    const l = tidyLayout({ id: "root", kind: "root", text: "T", full: "T", children: [] });
    expect(l.nodes).toHaveLength(1);
    expect(l.links).toEqual([]);
    expect(l.height).toBeGreaterThan(0);
  });

  it("lays out 200 leaves in under 16 ms", () => {
    const tree = randomTree(11, 200);
    tidyLayout(tree); // warm up
    const times = Array.from({ length: 7 }, () => {
      const t0 = performance.now();
      tidyLayout(tree);
      return performance.now() - t0;
    });
    expect(Math.min(...times)).toBeLessThan(16);
  });
});

describe("measureNode and wrapLines", () => {
  it("caps the width at 260 and wraps long text into at most four lines", () => {
    const long = "word ".repeat(60).trim();
    const lines = wrapLines(long);
    expect(lines.length).toBeLessThanOrEqual(4);
    const s = measureNode({ id: "x", kind: "leaf", text: long, full: long, children: [] });
    expect(s.w).toBeLessThanOrEqual(NODE_MAX_W);
  });

  it("ends the last kept line with an ellipsis when text is dropped", () => {
    const lines = wrapLines("word ".repeat(80).trim());
    expect(lines).toHaveLength(4);
    expect(lines[3]!.endsWith("…")).toBe(true);
    expect(wrapLines("short text")).toEqual(["short text"]);
  });

  it("cuts a word longer than a line instead of overflowing", () => {
    for (const l of wrapLines("a".repeat(200))) expect(Array.from(l).length).toBeLessThanOrEqual(30);
  });

  it("makes room for the Proposed chip", () => {
    const base = { id: "x", kind: "leaf" as const, text: "ship it", full: "ship it", children: [] };
    expect(measureNode({ ...base, proposed: true }).h).toBeGreaterThan(measureNode(base).h);
  });
});
