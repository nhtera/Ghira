// SPDX-License-Identifier: Apache-2.0
// The custom-vocabulary rules, the same as the desktop's (settings/logic.ts) and
// as Rust's set_vocabulary: trimmed, spaces collapsed, deduped without case or
// accents, capped at the core's `maxTerms`.
import { fold } from "./fold";

export type TermResult = {
  terms: string[];
  status: "added" | "empty" | "duplicate" | "full";
};

export function addTerm(terms: string[], input: string, max: number): TermResult {
  const term = input.trim().replace(/\s+/g, " ");
  if (!term) return { terms, status: "empty" };
  if (terms.some((t) => fold(t) === fold(term))) return { terms, status: "duplicate" };
  if (terms.length >= max) return { terms, status: "full" };
  return { terms: [...terms, term], status: "added" };
}
