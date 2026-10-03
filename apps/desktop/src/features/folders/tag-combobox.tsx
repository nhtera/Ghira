// SPDX-License-Identifier: Apache-2.0
// Type to find a tag or make one. Suggestions ignore accents ("hop" offers
// "Họp"); "Create “name”" appears only when no tag would be reused, because
// the core gives a lone accent variant back instead of making a second tag.
import { useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Icon, cn } from "@ghi/ui";
import type { TagRow } from "../../bindings";
import { MAX, foldName, hasAccents, nameKey } from "./organize";

export type Choice = { kind: "tag"; tag: TagRow } | { kind: "create"; name: string };

/** The options for what was typed: tags (not already used) that contain it, then maybe "Create". */
export function choicesFor(typed: string, tags: readonly TagRow[], exclude: ReadonlySet<string>): Choice[] {
  const text = typed.trim().replace(/\s+/g, " ");
  const folded = foldName(text);
  const free = tags.filter((t) => !exclude.has(t.gid));
  const matches = free.filter((t) => folded === "" || foldName(t.name).includes(folded));
  const out: Choice[] = matches.map((tag) => ({ kind: "tag", tag }));
  if (text === "") return out;
  const sameAll = tags.filter((t) => foldName(t.name) === folded);
  const exact = tags.some((t) => nameKey(t.name) === nameKey(text));
  // No "Create" when a tag with this exact name exists (even if already on the meeting), or when
  // the typed name has no accents and exactly one accent variant exists: the core reuses that one.
  // With accents typed ("Hộp" while only "Họp" exists) it is a new tag.
  if (!exact && !(!hasAccents(text) && sameAll.length === 1)) out.push({ kind: "create", name: text });
  return out;
}

export function TagCombobox({
  tags,
  exclude,
  onChoose,
  autoFocus,
}: {
  tags: readonly TagRow[];
  /** Tag gids the meeting already has. */
  exclude: ReadonlySet<string>;
  onChoose: (c: Choice) => void;
  autoFocus?: boolean;
}) {
  const { t } = useTranslation();
  const id = useId();
  const [typed, setTyped] = useState("");
  const [active, setActive] = useState(0);
  const choices = useMemo(() => choicesFor(typed, tags, exclude), [typed, tags, exclude]);
  const at = Math.min(active, Math.max(0, choices.length - 1));
  const pick = (c: Choice | undefined) => {
    if (!c) return;
    setTyped("");
    setActive(0);
    onChoose(c);
  };
  const label = (c: Choice) => (c.kind === "tag" ? c.tag.name : t("organize.createTag", { name: c.name }));
  return (
    <div className="flex w-64 flex-col gap-1.5">
      <input
        role="combobox"
        aria-expanded={choices.length > 0}
        aria-controls={`${id}-list`}
        aria-activedescendant={choices.length > 0 ? `${id}-${at}` : undefined}
        aria-autocomplete="list"
        aria-label={t("organize.tagName")}
        placeholder={t("organize.tagName")}
        autoFocus={autoFocus}
        value={typed}
        maxLength={MAX.tagName * 2}
        onChange={(e) => {
          setTyped(e.target.value);
          setActive(0);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setActive(Math.min(choices.length - 1, at + 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setActive(Math.max(0, at - 1));
          } else if (e.key === "Enter" && !e.nativeEvent.isComposing && e.keyCode !== 229) {
            // Enter confirms an IME candidate too; only a plain Enter picks.
            e.preventDefault();
            pick(choices[at]);
          }
        }}
        className="text-body h-8 rounded-ctl border border-ctl bg-surface px-2.5 text-ink focus-visible:outline-2 focus-visible:outline-accent"
      />
      <ul id={`${id}-list`} role="listbox" aria-label={t("organize.tags")} className="m-0 flex max-h-48 list-none flex-col gap-0.5 overflow-auto p-0">
        {choices.map((c, i) => (
          <li
            key={c.kind === "tag" ? c.tag.gid : "create"}
            id={`${id}-${i}`}
            role="option"
            aria-selected={i === at}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => pick(c)}
            className={cn("flex h-8 cursor-default items-center gap-2 rounded-seg px-2 text-[13px]", i === at && "bg-sunk")}
          >
            <Icon name={c.kind === "create" ? "add" : "label"} size={16} className="text-muted" />
            <span className="min-w-0 flex-1 truncate">{label(c)}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
