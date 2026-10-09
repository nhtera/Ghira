// SPDX-License-Identifier: Apache-2.0
// Horizontal tidy tree for the mind map: the root on the left, one column per
// depth, every parent centered on its children. A subtree owns the vertical
// band it needs, so nothing overlaps whatever the node heights; pure and
// deterministic (same tree, same boxes). Linear in the node count.
import type { notesTree } from "@ghi/ui";

type MapNode = notesTree.MapNode;

export type Size = { w: number; h: number };
export type Placed = { id: string; node: MapNode; depth: number; x: number; y: number; w: number; h: number };
export type Link = { from: string; to: string };
export type Layout = { nodes: Placed[]; links: Link[]; width: number; height: number };

export const NODE_MAX_W = 260;
export const NODE_MIN_W = 96;
const CHAR_W = 6.9;
export const PAD_X = 12;
export const LINE_H = 17;
export const PAD_Y = 9;
export const BADGE_H = 18;
export const GUTTER_W = 26;
export const COL_GAP = 56;
export const ROW_GAP = 10;
/** Characters per drawn line: what fits the width cap after the padding and the gutter. */
export const LINE_CHARS = Math.floor((NODE_MAX_W - 2 * PAD_X - GUTTER_W) / CHAR_W);

/** `text` broken at spaces into lines of at most `per` characters (a longer word is cut); what does not fit in `maxLines` ends the last line with an ellipsis. */
export function wrapLines(text: string, per = LINE_CHARS, maxLines = 4): string[] {
  const out: string[] = [];
  let cur = "";
  const len = (s: string) => Array.from(s).length;
  for (const word of text.split(" ").filter(Boolean)) {
    let w = word;
    while (len(w) > per) {
      if (cur) {
        out.push(cur);
        cur = "";
      }
      const chars = Array.from(w);
      out.push(chars.slice(0, per).join(""));
      w = chars.slice(per).join("");
    }
    if (!cur) cur = w;
    else if (len(cur) + 1 + len(w) <= per) cur += ` ${w}`;
    else {
      out.push(cur);
      cur = w;
    }
  }
  if (cur) out.push(cur);
  if (out.length <= maxLines) return out;
  const last = Array.from(out[maxLines - 1]!);
  return [...out.slice(0, maxLines - 1), `${last.slice(0, per - 1).join("").trimEnd()}…`];
}

/** The box of a node from its text (no DOM measuring: the layout stays pure and fast). */
export function measureNode(n: MapNode): Size {
  const lines = wrapLines(n.text);
  const longest = Math.max(1, ...lines.map((l) => Array.from(l).length));
  const gutter = n.kind === "section" || n.ownerGid !== undefined ? GUTTER_W : 0;
  const w = Math.min(NODE_MAX_W, Math.max(NODE_MIN_W, Math.ceil(longest * CHAR_W) + 2 * PAD_X + gutter));
  const h = Math.max(1, lines.length) * LINE_H + 2 * PAD_Y + (n.proposed ? BADGE_H : 0);
  return { w, h };
}

/** Lays `root` out, skipping the children of every id in `collapsed`. */
export function tidyLayout(root: MapNode, collapsed: ReadonlySet<string> = new Set(), size: (n: MapNode) => Size = measureNode): Layout {
  type T = { node: MapNode; depth: number; size: Size; kids: T[]; span: number };
  const build = (node: MapNode, depth: number): T => {
    const kids = collapsed.has(node.id) ? [] : node.children.map((c) => build(c, depth + 1));
    const s = size(node);
    const kidsSpan = kids.reduce((a, k) => a + k.span, 0) + Math.max(0, kids.length - 1) * ROW_GAP;
    return { node, depth, size: s, kids, span: Math.max(s.h, kidsSpan) };
  };
  const tree = build(root, 0);

  // One column per depth, as wide as its widest node.
  const colW: number[] = [];
  const widths = (t: T) => {
    colW[t.depth] = Math.max(colW[t.depth] ?? 0, t.size.w);
    t.kids.forEach(widths);
  };
  widths(tree);
  const colX: number[] = [];
  colW.reduce((x, w, d) => ((colX[d] = x), x + w + COL_GAP), 0);

  const nodes: Placed[] = [];
  const links: Link[] = [];
  /** Places `t` inside the band starting at `top`; returns the node's vertical center. */
  const place = (t: T, top: number): number => {
    const kidsSpan = t.kids.reduce((a, k) => a + k.span, 0) + Math.max(0, t.kids.length - 1) * ROW_GAP;
    let y = top + (t.span - kidsSpan) / 2;
    const centers: number[] = [];
    for (const k of t.kids) {
      links.push({ from: t.node.id, to: k.node.id });
      centers.push(place(k, y));
      y += k.span + ROW_GAP;
    }
    const wanted = centers.length ? (centers[0]! + centers[centers.length - 1]!) / 2 : top + t.span / 2;
    // Never leave the band: a neighbor's band starts right after it.
    const yTop = Math.min(Math.max(wanted - t.size.h / 2, top), top + t.span - t.size.h);
    nodes.push({ id: t.node.id, node: t.node, depth: t.depth, x: colX[t.depth]!, y: yTop, w: t.size.w, h: t.size.h });
    return yTop + t.size.h / 2;
  };
  place(tree, 0);
  nodes.sort((a, b) => a.depth - b.depth || a.y - b.y);
  return { nodes, links, width: colX[colW.length - 1]! + colW[colW.length - 1]!, height: tree.span };
}
