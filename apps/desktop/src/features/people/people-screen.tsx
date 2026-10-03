// SPDX-License-Identifier: Apache-2.0
// D8 People: the list (Me first) and one person's page. Voice data and the
// name in notes are separate things with separate confirms: deleting a voice
// profile keeps the name in notes, removing the name keeps the voice profile.
import { useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Avatar, EmptyState, Icon, cn } from "@ghi/ui";
import type { PersonRow } from "../../bindings";
import { errorText } from "./error-text";
import { PersonDetailView } from "./person-detail";
import { personName } from "./person-label";
import { usePeople } from "./queries";
import { VoiceBadge } from "./voice-badge";

function Row({ person, selected, onSelect }: { person: PersonRow; selected: boolean; onSelect: () => void }) {
  const { t } = useTranslation();
  const name = personName(person, t);
  return (
    <li>
      <button
        type="button"
        aria-current={selected ? "true" : undefined}
        onClick={onSelect}
        className={cn("grid w-full grid-cols-[32px_minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-px rounded-row px-2.5 py-2 text-left hover:bg-surface2", selected && "bg-surface2")}
      >
        <span className="row-span-2">
          <Avatar kind={person.isMe ? "me" : "person"} name={name} colorSlot={person.colorSlot} size="lg" />
        </span>
        <span className="truncate text-[14px] font-semibold">{name}</span>
        <span className="row-span-2">
          <VoiceBadge voice={person.voice} compact />
        </span>
        <span className="text-small truncate text-muted">
          {t("people.meetingsCount", { count: person.meetings })}
          {person.openActions > 0 && ` · ${t("people.openCount", { count: person.openActions })}`}
        </span>
      </button>
    </li>
  );
}

/** The screen's frame is the design's: the title sits above the list, which is a column of its own beside the person. */
export function PeopleBody({ title, subtitle }: { title?: string; subtitle?: string }) {
  const { t } = useTranslation();
  const q = usePeople();
  const [picked, setPicked] = useState<string | null>(null);
  const people = useMemo(() => q.data?.people ?? [], [q.data]);
  // A merged-away or removed person falls back to the first row.
  const selected = people.find((p) => p.gid === picked) ?? people[0];
  const heading = title && (
    <div data-tauri-drag-region>
      <h1 className="text-title m-0 mb-0.5 ml-2.5">{title}</h1>
      <p className="m-0 mb-3.5 ml-2.5 text-[12.5px] text-muted">{subtitle}</p>
    </div>
  );
  const alone = (body: ReactNode) => (
    <div className="h-full overflow-auto px-3.5 pt-5 pb-8">
      {heading}
      {body}
    </div>
  );
  if (q.isError) return alone(<p className="text-body ml-2.5 text-muted">{errorText(t, q.error.message)}</p>);
  if (!q.data) return alone(null);
  const onlyMe = people.every((p) => p.isMe);
  if (people.length === 0 || (onlyMe && (people[0]?.meetings ?? 0) === 0)) return alone(<EmptyState kind="people" className="mt-10" />);

  return (
    <div className="grid h-full min-h-0 grid-cols-[340px_minmax(0,1fr)]">
      <nav aria-label={t("people.list")} className="min-h-0 overflow-auto border-r border-line px-3.5 pt-5 pb-8">
        {heading}
        <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
          {people.map((p) => (
            <Row key={p.gid} person={p} selected={p.gid === selected?.gid} onSelect={() => setPicked(p.gid)} />
          ))}
        </ul>
        {/* The unknown-voices queue and "save a voice profile" for others are for `thirdParty`, always off in this build. */}
      </nav>
      <div className="min-h-0 overflow-auto px-9 pt-6 pb-12">
        {selected ? <PersonDetailView key={selected.gid} gid={selected.gid} people={people} thirdParty={q.data.thirdParty} onSelect={setPicked} /> : <Icon name="group" size={32} className="text-faint" />}
      </div>
    </div>
  );
}
