// SPDX-License-Identifier: Apache-2.0
// Built-in template names by id (copy keys; a template the UI doesn't know
// keeps the name the core gives it).
import type { TFunction } from "i18next";

const KEYS: Record<string, string> = {
  general: "library.templates.general",
  one_on_one: "library.templates.oneOnOne",
  client: "library.templates.clientCall",
  standup: "library.templates.standup",
  interview: "meeting.templates.interview",
  lecture: "meeting.templates.lecture",
  sales: "meeting.templates.sales",
};

export const DEFAULT_TEMPLATE = "general";

export function templateName(
  id: string | null | undefined,
  t: TFunction,
  fallback?: string,
): string {
  const key = KEYS[id ?? DEFAULT_TEMPLATE];
  return key ? (t as (k: string) => string)(key) : (fallback ?? id ?? "");
}
