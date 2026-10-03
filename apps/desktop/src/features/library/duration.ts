// SPDX-License-Identifier: Apache-2.0
// A meeting's length in words for a library row: "42 min", "1 h 34 min".
import type { TFunction } from "i18next";

export function durationLabel(t: TFunction, ms: number): string {
  const total = Math.max(1, Math.round(ms / 60_000));
  const hours = Math.floor(total / 60);
  const minutes = total % 60;
  if (hours === 0) return t("library.duration.minutes", { minutes });
  if (minutes === 0) return t("library.duration.hours", { hours });
  return t("library.duration.hoursMinutes", { hours, minutes });
}
