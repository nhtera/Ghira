// SPDX-License-Identifier: Apache-2.0
// Action items: tick, edit the text, pick an owner, see the due words, delete,
// and add. The owner is color + initial + name, never color alone.
import { Avatar, Icon, Menu, cn, type MenuItem } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import type { ActionItemView } from "../../bindings";
import { CitationGroup } from "../citation/citation-link";
import { findSpeaker, speakerDisplay } from "../meeting/speaker-display";
import { AutoTextarea } from "./auto-textarea";
import { Provenance } from "./block-row";
import { useNotesContext } from "./notes-context";

function OwnerMenu({ item }: { item: ActionItemView }) {
  const { t } = useTranslation();
  const { speakers, edit } = useNotesContext();
  const owner = findSpeaker(speakers, item.ownerSpeakerGid);
  const who = owner ? speakerDisplay(owner, t) : null;
  const items: MenuItem[] = [
    ...speakers
      .filter((s) => !s.notPerson)
      .map((s): MenuItem => ({
        label: speakerDisplay(s, t).name,
        icon: s.gid === item.ownerSpeakerGid ? "check" : undefined,
        onSelect: () => void edit.setOwner(item.gid, s.gid),
      })),
    { kind: "separator" },
    {
      label: t("notes.noOwner"),
      icon: item.ownerSpeakerGid ? undefined : "check",
      onSelect: () => void edit.setOwner(item.gid, null),
    },
  ];
  return (
    <Menu
      label={t("notes.ownerMenu")}
      align="start"
      items={items}
      trigger={
        <button
          type="button"
          aria-label={t("notes.ownerOf", {
            owner: who?.name ?? t("notes.unassigned"),
          })}
          className="inline-flex h-[26px] items-center gap-1.5 rounded-full border border-line py-0 pr-2.5 pl-[3px] text-[12.5px] font-semibold text-ink hover:bg-sunk"
        >
          {who ? (
            <Avatar
              kind={who.isMe ? "me" : "person"}
              name={who.name}
              initial={who.initial}
              colorSlot={who.colorSlot}
              size="sm"
            />
          ) : (
            <Avatar kind="unknown" size="sm" />
          )}
          {who ? who.name : t("notes.unassigned")}
        </button>
      }
    />
  );
}

function ActionRow({ item }: { item: ActionItemView }) {
  const { t } = useTranslation();
  const { meeting, speakers, audioAvailable, edit } = useNotesContext();
  const provenance =
    item.origin === "user"
      ? "user"
      : item.origin === "aiEdited"
        ? "edited"
        : "ai";
  return (
    <li
      data-action={item.gid}
      data-done={item.done ? "true" : undefined}
      className="flex flex-col gap-1 py-1"
    >
      <div className="flex items-start gap-2">
        <button
          type="button"
          role="checkbox"
          aria-checked={item.done}
          aria-label={t("notes.markDone")}
          aria-describedby={`action-text-${item.gid}`}
          onClick={() => void edit.setDone(item.gid, !item.done)}
          className="group grid size-6 shrink-0 place-items-center rounded-seg"
        >
          <span
            className={cn(
              "grid size-[18px] place-items-center rounded-[5px] border-2 text-on-accent",
              item.done ? "border-accent bg-accent" : "border-ctl",
            )}
          >
            {item.done && <Icon name="check" size={14} />}
          </span>
        </button>
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-1">
          <div className="min-w-0 flex-1 basis-56">
            <AutoTextarea
              value={item.text}
              id={`action-text-${item.gid}`}
              keepOnEmpty
              label={t("notes.actionLabel")}
              trailing={
                item.citations.length > 0 && (
                  <span className="ml-1.5 align-[2px]">
                    <CitationGroup citations={item.citations} speakers={speakers} audioAvailable={audioAvailable} meeting={meeting} />
                  </span>
                )
              }
              onCommit={(text) =>
                text.trim() &&
                text.trim() !== item.text &&
                void edit.editAction(item.gid, text.trim())
              }
              className={cn(
                "font-serif text-notes",
                item.done
                  ? "text-muted line-through"
                  : item.origin === "user"
                    ? "text-ink"
                    : "text-ai",
              )}
            />
          </div>
          <OwnerMenu item={item} />
          {item.dueText && (
            <span className="inline-flex items-center gap-1 text-[12.5px] text-muted">
              <Icon name="schedule" size={14} />
              {item.dueText}
            </span>
          )}
          <button
            type="button"
            aria-label={t("common.delete")}
            onClick={() => void edit.deleteAction(item.gid)}
            className="grid size-6 place-items-center rounded-seg text-muted hover:bg-sunk hover:text-ink"
          >
            <Icon name="delete" size={16} />
          </button>
        </div>
      </div>
      {/* The legend above the notes says what the two colors mean; an edited item still says it. */}
      <span className={cn("ml-8", provenance !== "edited" && "sr-only")}>
        <Provenance kind={provenance} />
      </span>
    </li>
  );
}

export function ActionItems({ items }: { items: ActionItemView[] }) {
  const { t } = useTranslation();
  const { edit } = useNotesContext();
  return (
    <div className="flex flex-col gap-1">
      {items.length > 0 && (
        <ul className="m-0 flex list-none flex-col p-0">
          {items.map((a) => (
            <ActionRow key={a.gid} item={a} />
          ))}
        </ul>
      )}
      <div className="ml-8">
        <AutoTextarea
          value=""
          clearOnEnter
          label={t("notes.addAction")}
          placeholder={t("notes.addAction")}
          className="text-body text-ink"
          onCommit={(text) => text.trim() && void edit.addAction(text.trim())}
          onEnter={() => undefined}
        />
      </div>
    </div>
  );
}
