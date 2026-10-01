// SPDX-License-Identifier: Apache-2.0
// Speaker avatar: initial on the speaker color (never color alone: the name
// is always next to it or in `label`). Person · Unknown voice · Me · Group.
import { useTranslation } from "react-i18next";
import { cn } from "../../utils/cn";

export type AvatarSize = "sm" | "md" | "lg" | "xl";
export type AvatarKind = "person" | "unknown" | "me" | "group";

const SIZE: Record<AvatarSize, string> = {
  sm: "size-5 text-[10px]",
  md: "size-6 text-[11px]",
  lg: "size-8 text-[12px]",
  xl: "size-10 text-[14px]",
};

// "Me" / "Tôi" is three letters: a notch smaller so it fits the small circles.
const ME_SIZE: Record<AvatarSize, string> = {
  sm: "size-5 text-[8px]",
  md: "size-6 text-[9px]",
  lg: "size-8 text-[12px]",
  xl: "size-10 text-[14px]",
};

const segmenter = typeof Intl !== "undefined" && "Segmenter" in Intl ? new Intl.Segmenter(undefined, { granularity: "grapheme" }) : null;

/** First letter of the first word, so "Nguyễn Văn An" → "N", "đặng" → "Đ"; keeps diacritics. */
export function initialOf(name: string): string {
  const word = name.normalize("NFC").trim().split(/\s+/)[0] ?? "";
  const letters = word.replace(/^[^\p{L}\p{N}]+/u, "");
  const first = segmenter ? segmenter.segment(letters)[Symbol.iterator]().next().value?.segment : Array.from(letters)[0];
  return (first ?? "").toLocaleUpperCase();
}

export type AvatarProps = {
  kind?: AvatarKind;
  /** Person name; the initial is derived from it unless `initial` is set. */
  name?: string;
  initial?: string;
  /** Speaker color slot 1..8; 0 is the neutral "Others" color. */
  colorSlot?: number;
  /** Group: how many people ("+3"). */
  count?: number;
  size?: AvatarSize;
  /** Accessible name; without it the avatar is decorative. */
  label?: string;
  className?: string;
};

export function Avatar({ kind = "person", name, initial, colorSlot = 1, count = 0, size = "lg", label, className }: AvatarProps) {
  const { t } = useTranslation();
  const slot = Math.min(8, Math.max(0, Math.trunc(colorSlot)));
  const base = cn(
    "inline-grid shrink-0 place-items-center rounded-full border-[1.5px] font-bold leading-none select-none",
    kind === "me" ? ME_SIZE[size] : SIZE[size],
    className,
  );
  const a11y = label ? ({ role: "img", "aria-label": label } as const) : ({ "aria-hidden": true } as const);

  if (kind === "unknown") {
    return (
      <span {...a11y} data-kind="unknown" className={cn(base, "border-dashed border-line2 text-muted")}>
        {"?"}
      </span>
    );
  }
  if (kind === "group") {
    return (
      <span {...a11y} data-kind="group" className={cn(base, "border-line2 bg-sunk text-ink")}>
        {t("common.plusCount", { count })}
      </span>
    );
  }
  const neutral = slot === 0;
  const text = kind === "me" ? t("speakers.me") : (initial ?? (name ? initialOf(name) : ""));
  return (
    <span
      {...a11y}
      data-kind={kind}
      className={cn(base, "border-transparent", neutral ? "bg-sunk text-ink border-line2" : "text-on-s", kind === "me" && "tracking-tighter")}
      style={neutral ? undefined : { background: `var(--s${slot})` }}
    >
      {text}
    </span>
  );
}
