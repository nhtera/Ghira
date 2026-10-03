// SPDX-License-Identifier: Apache-2.0
/** "1.2 GB", "340 MB": localized by Intl (unit names come from the platform, not our locale files). */
export function formatBytes(
  bytes: number | null | undefined,
  lang: string,
): string {
  if (bytes == null) return "";
  const gb = bytes >= 1e9;
  return new Intl.NumberFormat(lang, {
    style: "unit",
    unit: gb ? "gigabyte" : "megabyte",
    unitDisplay: "short",
    maximumFractionDigits: gb ? 1 : 0,
  }).format(gb ? bytes / 1e9 : bytes / 1e6);
}
