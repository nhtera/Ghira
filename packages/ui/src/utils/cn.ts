// SPDX-License-Identifier: Apache-2.0
/** Joins class names, skipping falsy ones. */
export function cn(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(" ");
}
