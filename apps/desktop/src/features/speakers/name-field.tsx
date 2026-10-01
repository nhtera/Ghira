// SPDX-License-Identifier: Apache-2.0
// A name box with the people already known (combobox): type, or arrow to a
// match, Enter. A name that isn't known yet is offered as a new person.
import { useId, useMemo, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@ghi/ui";

/** Known names containing what was typed (case- and accent-insensitive is the library's job; this is a quick local filter). */
export function matchNames(names: string[], query: string, limit = 5): string[] {
  const q = query.trim().toLowerCase();
  const hits = q ? names.filter((n) => n.toLowerCase().includes(q)) : names;
  return hits.slice(0, limit);
}

export function NameField({ names, onSubmit, label, placeholder }: { names: string[]; onSubmit: (name: string, known: boolean) => void; label: string; placeholder: string }) {
  const { t } = useTranslation();
  const id = useId();
  const [text, setText] = useState("");
  const [active, setActive] = useState(-1);
  const options = useMemo(() => matchNames(names, text), [names, text]);
  const typed = text.trim();
  const isNew = Boolean(typed) && !names.some((n) => n.toLowerCase() === typed.toLowerCase());
  const listId = `${id}-list`;

  const submit = (name: string) => {
    const value = name.trim();
    if (value) onSubmit(value, names.some((n) => n.toLowerCase() === value.toLowerCase()));
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!options.length) return;
      const step = e.key === "ArrowDown" ? 1 : -1;
      // -1 is "the typed text"; arrows walk the matches and wrap back to it.
      setActive((a) => {
        const n = a + step;
        return n >= options.length ? -1 : n < -1 ? options.length - 1 : n;
      });
    } else if (e.key === "Enter" && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit(active >= 0 && options[active] ? options[active] : text);
    }
  };

  return (
    <div className="flex flex-col gap-1.5">
      <input
        autoFocus
        role="combobox"
        aria-expanded={options.length > 0}
        aria-controls={listId}
        aria-activedescendant={active >= 0 ? `${id}-o${active}` : undefined}
        aria-autocomplete="list"
        aria-label={label}
        placeholder={placeholder}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          setActive(-1);
        }}
        onKeyDown={onKeyDown}
        className="text-body h-9 w-full rounded-ctl border border-ctl bg-surface px-3"
      />
      <ul id={listId} role="listbox" aria-label={t("speakers.inPeople")} className="m-0 flex list-none flex-col p-0 empty:hidden">
        {options.map((n, i) => (
          <li key={n} id={`${id}-o${i}`} role="option" aria-selected={i === active} onMouseDown={(e) => e.preventDefault()} onClick={() => submit(n)} className={cn("text-body flex h-8 cursor-default items-center rounded-seg px-2", i === active ? "bg-accent-soft text-accent" : "hover:bg-sunk")}>
            {n}
          </li>
        ))}
        {isNew && (
          <li role="option" aria-selected={false} onMouseDown={(e) => e.preventDefault()} onClick={() => submit(typed)} className="text-body flex h-8 cursor-default items-center gap-1.5 rounded-seg px-2 hover:bg-sunk">
            <b className="font-semibold">{typed}</b>
            <span className="text-small text-muted">{t("speakers.newPerson")}</span>
          </li>
        )}
      </ul>
    </div>
  );
}
