// SPDX-License-Identifier: Apache-2.0
// A keycap chip for a shortcut: "⌘ ⇧ R" on a Mac, "Ctrl + Shift + R" on Windows.
// Pass the label from `shortcutLabel()` ("⌘⇧R" or "Ctrl+Shift+R"); the keys are
// spaced apart so the chip reads as a chord. Text only, so it also reads aloud.
import { cn } from "../../utils/cn";

export type KbdProps = {
  /** The shortcut label: mac symbols run together ("⌘⇧R"), Windows keys joined by "+". */
  shortcut: string;
  /** `md` sits inline under a line of text; `lg` is the onboarding hero chip. */
  size?: "md" | "lg";
  className?: string;
};

/** The keys of a label: split on "+", or one symbol each when there is no "+". */
export function kbdKeys(shortcut: string): string[] {
  return shortcut.includes("+") ? shortcut.split("+") : Array.from(shortcut);
}

export function Kbd({ shortcut, size = "md", className }: KbdProps) {
  const win = shortcut.includes("+");
  const keys = kbdKeys(shortcut);
  return (
    <kbd
      data-size={size}
      className={cn(
        "text-mono inline-flex items-center border border-line2 bg-surface2 text-ink",
        win ? "gap-1.5" : "gap-2",
        size === "lg" ? "rounded-[10px] px-4 py-2.5 text-[18px] font-medium" : "rounded-md px-2 py-0.5 text-[12px]",
        className,
      )}
    >
      {keys.map((k, i) => (
        <span key={i} className="inline-flex items-center gap-1.5">
          {i > 0 && win && (
            <span aria-hidden className="text-faint">
              +
            </span>
          )}
          {k}
        </span>
      ))}
    </kbd>
  );
}
