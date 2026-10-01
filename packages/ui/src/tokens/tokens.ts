// SPDX-License-Identifier: Apache-2.0
// Design tokens (tokens.json) for code that needs the values themselves:
// tests, canvas drawing, the speaker palette order.
import tokens from "./tokens.json";

export type Theme = "light" | "dark";
export type ColorToken = keyof typeof tokens.color.light;
export type SpeakerSlot = "s1" | "s2" | "s3" | "s4" | "s5" | "s6" | "s7" | "s8";

export const colors: Record<Theme, Record<ColorToken, string>> = tokens.color;
export const radius = tokens.radius;
export const typeScale = tokens.type;

/**
 * Palette slots in assignment order: the first four stay apart under the
 * common color-vision deficiencies (design notes, "Speaker palette").
 */
export const speakerOrder = tokens.speakerOrder as SpeakerSlot[];

/** The slot for the n-th speaker (0-based), cycling after eight. */
export function speakerSlot(n: number): SpeakerSlot {
  return speakerOrder[((n % speakerOrder.length) + speakerOrder.length) % speakerOrder.length];
}
