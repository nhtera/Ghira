// SPDX-License-Identifier: Apache-2.0
// Pure helpers of the Sync UI: the pairing countdown and "4 min ago".

/** Used when the core gives no expiry (the real core: 120 s). */
export const DEFAULT_PAIR_TTL_MS = 120_000;

/** `m:ss`, rounded up so "0:00" only shows once the code is dead. */
export function formatCountdown(ms: number): string {
  const s = Math.max(0, Math.ceil(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** "4 min. ago", "2 hr. ago", "3 days ago" in the app language; under a minute is `justNow`. */
export function whenAgo(ms: number, now: number, lang: string, justNow: string): string {
  const diff = Math.max(0, now - ms);
  const min = Math.floor(diff / 60_000);
  if (min < 1) return justNow;
  const rtf = new Intl.RelativeTimeFormat(lang, { numeric: "always", style: "short" });
  if (min < 60) return rtf.format(-min, "minute");
  if (min < 60 * 24) return rtf.format(-Math.floor(min / 60), "hour");
  return rtf.format(-Math.floor(min / (60 * 24)), "day");
}

/** The offer's SVG as an `<img>` source, or null when it is not an SVG document. */
export function qrDataUrl(svg: string): string | null {
  const s = svg.trimStart();
  return s.startsWith("<svg") || s.startsWith("<?xml") ? `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}` : null;
}

const KNOWN_ERRORS = ["unreachable", "refused", "upgradeRequired", "storageFull", "locked", "internal"] as const;
export type KnownSyncError = (typeof KNOWN_ERRORS)[number];

/** An unknown code is worded as `internal`. */
export function errorKey(code: string): KnownSyncError {
  return (KNOWN_ERRORS as readonly string[]).includes(code) ? (code as KnownSyncError) : "internal";
}

const TRANSFER_ERRORS = ["passphrase_short", "wrong_passphrase", "not_an_export"] as const;
export type TransferError = (typeof TRANSFER_ERRORS)[number];

/** The core's error code for a file export or import, or null for any other message. */
export function transferErrorKey(error: string): TransferError | null {
  return (TRANSFER_ERRORS as readonly string[]).includes(error) ? (error as TransferError) : null;
}
