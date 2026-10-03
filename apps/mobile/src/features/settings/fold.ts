// SPDX-License-Identifier: Apache-2.0
/** Case and accents ignored, like Rust's check of the delete phrase (đ -> d). */
export function fold(s: string): string {
  return s
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .replace(/đ/gi, "d")
    .trim()
    .toLowerCase();
}

/** The phrases Rust accepts for "delete everything" (privacy_cmd::DELETE_PHRASES). */
export const DELETE_PHRASES = ["DELETE", "XÓA"] as const;

export const isDeletePhrase = (typed: string): boolean =>
  DELETE_PHRASES.some((p) => fold(p) === fold(typed));
