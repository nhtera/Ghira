// SPDX-License-Identifier: Apache-2.0
// Locale formats from brief §8: Intl only, no date library.
import type { Locale } from "./index";

const tag = (l: Locale) => (l === "vi" ? "vi-VN" : "en-US");

/** 28/09/2026 (vi) · Sep 28, 2026 (en). */
export function formatDate(d: Date | number, l: Locale): string {
  return new Intl.DateTimeFormat(tag(l), l === "vi" ? { day: "2-digit", month: "2-digit", year: "numeric" } : { month: "short", day: "numeric", year: "numeric" }).format(d);
}

/** 14:05 (vi, 24 h) · 2:05 PM (en). */
export function formatTime(d: Date | number, l: Locale): string {
  return new Intl.DateTimeFormat(tag(l), { hour: "numeric", minute: "2-digit", hour12: l !== "vi" }).format(d);
}

/** 42:07 or 1:02:07 (meeting time from milliseconds); `pad` gives 00:04 like the design's live clock. */
export function formatClock(ms: number, opts?: { pad?: boolean }): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const mm = String(Math.floor((s % 3600) / 60)).padStart(h || opts?.pad ? 2 : 1, "0");
  const ss = String(s % 60).padStart(2, "0");
  return h ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/** 1.2 GB / 1,2 GB, 340 MB (decimal units, the locale's separator). */
export function formatBytes(bytes: number | null | undefined, l: Locale | string): string {
  if (bytes == null || !Number.isFinite(bytes)) return "";
  const gb = bytes >= 1e9;
  const value = gb ? bytes / 1e9 : Math.max(1, bytes / 1e6);
  const digits = gb && value < 10 ? 1 : 0;
  const n = new Intl.NumberFormat(l === "vi" ? "vi-VN" : l === "en" ? "en-US" : l, { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(value);
  return `${n} ${gb ? "GB" : "MB"}`;
}
