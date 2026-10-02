// SPDX-License-Identifier: Apache-2.0
// The block editor's one control: a native textarea that grows with its text
// (no rich-text editor: plain text only, RT-6). Saves on blur and after a
// pause in typing, never while an IME composition is open, and never treats
// the Enter that confirms a composition as "new note".
import { cn } from "@ghi/ui";
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";

const SAVE_AFTER_MS = 800;

export type AutoTextareaProps = {
  value: string;
  onCommit: (text: string) => void;
  /** Enter with the caret at the end (Shift+Enter always makes a line break). */
  onEnter?: () => void;
  /** Backspace in an empty field. */
  onBackspaceEmpty?: () => void;
  label: string;
  placeholder?: string;
  autoFocus?: boolean;
  className?: string;
  /** The "add" row: commits on Enter or blur only (never mid-typing), then clears. */
  clearOnEnter?: boolean;
  /** Emptying the field and leaving it restores the text instead of committing "". */
  keepOnEmpty?: boolean;
  id?: string;
};

export function AutoTextarea({
  value,
  onCommit,
  onEnter,
  onBackspaceEmpty,
  label,
  placeholder,
  autoFocus,
  className,
  clearOnEnter,
  keepOnEmpty,
  id,
}: AutoTextareaProps) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const [draft, setDraft] = useState(value);
  const saved = useRef(value);
  const composing = useRef(false);
  const timer = useRef<number>(0);
  const latest = useRef({ draft, onCommit });
  useLayoutEffect(() => {
    latest.current = { draft, onCommit };
  });

  // Follow the stored text unless the user has unsaved changes here.
  useEffect(() => {
    // Compared trimmed: parents store trimmed text, and a trailing space or line break in progress must survive.
    if (
      value.trim() !== saved.current.trim() &&
      latest.current.draft === saved.current
    )
      setDraft(value);
    if (value.trim() !== saved.current.trim()) saved.current = value;
  }, [value]);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [draft]);

  useEffect(() => {
    if (autoFocus) ref.current?.focus();
  }, [autoFocus]);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  const commit = (text: string) => {
    window.clearTimeout(timer.current);
    if (text === saved.current) return;
    saved.current = text;
    latest.current.onCommit(text);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    const ime =
      composing.current || e.nativeEvent.isComposing || e.keyCode === 229;
    if (ime) return;
    const el = e.currentTarget;
    if (
      e.key === "Enter" &&
      !e.shiftKey &&
      onEnter &&
      el.selectionStart === el.value.length &&
      el.selectionEnd === el.value.length
    ) {
      e.preventDefault();
      commit(draft);
      if (clearOnEnter) {
        saved.current = "";
        setDraft("");
      }
      onEnter();
    } else if (e.key === "Backspace" && draft === "" && onBackspaceEmpty) {
      e.preventDefault();
      onBackspaceEmpty();
    }
  };

  return (
    <textarea
      ref={ref}
      id={id}
      rows={1}
      aria-label={label}
      placeholder={placeholder}
      value={draft}
      spellCheck
      onChange={(e) => {
        setDraft(e.target.value);
        window.clearTimeout(timer.current);
        const text = e.target.value;
        // Never mid-typing for the add row, and never an empty field (retyping is not deleting).
        if (clearOnEnter || !text.trim()) return;
        timer.current = window.setTimeout(
          () => !composing.current && commit(text),
          SAVE_AFTER_MS,
        );
      }}
      onCompositionStart={() => (composing.current = true)}
      onCompositionEnd={(e) => {
        composing.current = false;
        setDraft(e.currentTarget.value);
      }}
      onKeyDown={onKeyDown}
      onBlur={() => {
        if (clearOnEnter) {
          if (draft.trim()) commit(draft);
          saved.current = "";
          setDraft("");
        } else if (keepOnEmpty && !draft.trim()) {
          window.clearTimeout(timer.current);
          saved.current = value;
          setDraft(value);
        } else commit(draft);
      }}
      className={cn(
        "m-0 block w-full resize-none overflow-hidden rounded-seg border-0 bg-transparent p-0 outline-none placeholder:text-muted focus-visible:ring-2 focus-visible:ring-accent",
        className,
      )}
    />
  );
}
