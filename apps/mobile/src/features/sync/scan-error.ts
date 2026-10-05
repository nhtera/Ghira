// SPDX-License-Identifier: Apache-2.0
// Why a scan did not pair, as the copy key to show. The camera and the code
// check are native; what reaches the web side is a code (a failed scan start)
// or a sync error event. Unknown codes read as "not a code".
export type ScanFailure = "cameraOff" | "invalid" | "expired" | "notFound" | "localNetwork" | "storageFull" | "upgradeRequired" | "locked" | "internal";

const KNOWN: readonly ScanFailure[] = ["cameraOff", "invalid", "expired", "notFound", "localNetwork", "storageFull", "upgradeRequired", "locked", "internal"];

export function scanFailure(code: string): ScanFailure {
  // The sync error codes: no path to the computer, or the computer refused the code.
  if (code === "unreachable") return "notFound";
  if (code === "refused") return "expired";
  return (KNOWN as readonly string[]).includes(code) ? (code as ScanFailure) : "invalid";
}

/** The copy key under `mobile.sync.`. */
export const scanFailureKey = (f: ScanFailure) =>
  (["cameraOff", "invalid", "expired", "notFound", "localNetwork"].includes(f) ? `scan.${f}` : `error.${f}`) as
    | "scan.cameraOff"
    | "scan.invalid"
    | "scan.expired"
    | "scan.notFound"
    | "scan.localNetwork"
    | "error.storageFull"
    | "error.upgradeRequired"
    | "error.locked"
    | "error.internal";

/** Failures that point to Settings (camera and local network are permissions). */
export const needsSettings = (f: ScanFailure) => f === "cameraOff" || f === "localNetwork";

/** Failures where guidance about Personal Hotspot helps (the computer was not reachable). */
export const wantsHotspot = (f: ScanFailure) => f === "notFound";

/** The copy key under `mobile.sync.error.` for the last failed session (unknown codes read as "internal"). */
export function syncErrorKey(code: string): "unreachable" | "refused" | "upgradeRequired" | "storageFull" | "locked" | "internal" {
  return (["unreachable", "refused", "upgradeRequired", "storageFull", "locked"] as const).find((c) => c === code) ?? "internal";
}
