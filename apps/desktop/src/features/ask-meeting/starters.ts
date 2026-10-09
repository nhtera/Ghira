// SPDX-License-Identifier: Apache-2.0
// Starter questions of the per-meeting Ask panel: the meeting's own open
// questions (notes blocks of kind `question`), only those that really are
// questions (end in "?" or the full-width "？" some Vietnamese and CJK input
// methods type), then the static ones to make up three.
export const STARTER_MAX = 3;

const asks = (text: string) => /[?？]$/.test(text);

/** Up to `max` distinct open questions, in the notes' order. */
export function openQuestionStarters(blocks: readonly { kind: string; text: string }[], max = STARTER_MAX): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const b of blocks) {
    const text = b.text.replace(/\s+/g, " ").trim();
    if (b.kind !== "question" || !asks(text) || seen.has(text.toLowerCase())) continue;
    seen.add(text.toLowerCase());
    out.push(text);
    if (out.length === max) break;
  }
  return out;
}
