// SPDX-License-Identifier: Apache-2.0
// A snippet with its matches marked. The core sends `[start, end)` ranges in
// UTF-16 units of the original text, so the highlight keeps the user's accents.
import type { ReactNode } from "react";

export type Piece = { text: string; hit: boolean };

/** Splits `text` at the (sorted, clamped, non-overlapping) ranges. */
export function splitHighlights(
  text: string,
  ranges: ReadonlyArray<readonly [number, number]>,
): Piece[] {
  const out: Piece[] = [];
  let at = 0;
  for (const [a, b] of [...ranges].sort((x, y) => x[0] - y[0])) {
    const start = Math.max(a, at);
    const end = Math.min(b, text.length);
    if (end <= start) continue;
    if (start > at) out.push({ text: text.slice(at, start), hit: false });
    out.push({ text: text.slice(start, end), hit: true });
    at = end;
  }
  if (at < text.length) out.push({ text: text.slice(at), hit: false });
  return out;
}

export function Highlighted({
  text,
  ranges,
}: {
  text: string;
  ranges: ReadonlyArray<readonly [number, number]>;
}): ReactNode {
  return splitHighlights(text, ranges).map((p, i) =>
    p.hit ? (
      <mark
        key={i}
        className="rounded-[0.1875rem] bg-warn-soft font-semibold text-ink"
      >
        {p.text}
      </mark>
    ) : (
      p.text
    ),
  );
}
