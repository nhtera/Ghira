// SPDX-License-Identifier: Apache-2.0

import { type RefObject, useEffect, useRef, useState } from "react";
import { landing } from "@/content/landing";

const t = landing.get;

/**
 * Copies a code block. Without clipboard access (an insecure context, a
 * blocked permission) it selects the block instead, so Cmd+C works.
 */
export function CopyButton({ text, target }: { text: string; target: RefObject<HTMLElement | null> }) {
  const [label, setLabel] = useState<string>(t.copy);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      setLabel(t.copied);
    } catch {
      const el = target.current;
      const selection = getSelection();
      if (el && selection) {
        const range = document.createRange();
        range.selectNodeContents(el);
        selection.removeAllRanges();
        selection.addRange(range);
      }
      setLabel(t.selected);
    }
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setLabel(t.copy), 1600);
  }

  return (
    <button className="copy-btn" type="button" onClick={copy}>
      {label}
    </button>
  );
}
