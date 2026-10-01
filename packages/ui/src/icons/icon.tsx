// SPDX-License-Identifier: Apache-2.0
// One icon component for both platforms: Material Symbols Rounded on mac,
// Fluent System Icons on Windows (design notes). Decorative unless `label`.
import { usePlatform } from "../platform/platform";
import { FLUENT, MATERIAL, type IconName } from "./icon-data";

export type { IconName } from "./icon-data";

export type IconProps = {
  name: IconName;
  /** Rendered size in px (square). */
  size?: number;
  /** Accessible name; without it the icon is hidden from assistive tech. */
  label?: string;
  className?: string;
};

export function Icon({ name, size = 18, label, className }: IconProps) {
  const platform = usePlatform();
  const fluent = platform === "win" ? FLUENT[name] : undefined;
  const d = fluent ?? MATERIAL[name];
  return (
    <svg
      width={size}
      height={size}
      viewBox={fluent ? "0 0 20 20" : "0 -960 960 960"}
      fill="currentColor"
      className={className}
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      focusable="false"
      data-icon={name}
    >
      {d.map((p, i) => (
        <path key={i} d={p} />
      ))}
    </svg>
  );
}
