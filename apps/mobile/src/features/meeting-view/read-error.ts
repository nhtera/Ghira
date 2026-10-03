// SPDX-License-Identifier: Apache-2.0
export type ReadError = "locked" | "starting" | "missing" | "failed";

/** Why a read failed, from the core's message ("the app is locked", "the app is starting", ...). */
export function classifyError(message: string): ReadError {
  const m = message.toLowerCase();
  if (m.includes("locked")) return "locked";
  if (m.includes("starting")) return "starting";
  if (
    m.includes("not found") ||
    m.includes("no such") ||
    m.includes("unknown meeting")
  )
    return "missing";
  return "failed";
}
