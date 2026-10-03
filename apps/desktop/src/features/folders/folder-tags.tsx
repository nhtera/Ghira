// SPDX-License-Identifier: Apache-2.0
// The meeting header's folder and tags: the folder chip opens the move dialog,
// tags are chips with × to remove, and "Add tag…" is a combobox that can make
// a tag on the spot. Names are text nodes.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Icon, Popover } from "@ghi/ui";
import { useMeetings } from "../library/use-meetings";
import { MoveDialog } from "./move-dialog";
import { useFolders, useOrganizeActions, useTags } from "./organize";
import { TagCombobox, type Choice } from "./tag-combobox";

const chip = "inline-flex h-6 items-center gap-1 rounded-full border border-line2 px-2 text-[12px] text-muted";

export function FolderTags({ meeting }: { meeting: string }) {
  const { t } = useTranslation();
  const row = useMeetings().rows.find((r) => r.gid === meeting);
  const folders = useFolders().data ?? [];
  const tags = useTags().data ?? [];
  const actions = useOrganizeActions();
  const [moving, setMoving] = useState(false);
  const [adding, setAdding] = useState(false);
  if (!row) return null;
  const folder = folders.find((f) => f.gid === row.folder);
  const have = new Set(row.tags.map((x) => x.gid));
  const choose = async (c: Choice) => {
    const tag = c.kind === "tag" ? c.tag : await actions.createTag(c.name);
    if (tag && (await actions.tag([meeting], tag))) setAdding(false);
  };
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <button type="button" onClick={() => setMoving(true)} aria-label={`${t("organize.folder")}: ${folder?.name ?? t("organize.noFolder")}`} className={chip + " hover:border-accent hover:text-accent"}>
        <Icon name="folder" size={14} />
        {folder?.name ?? t("organize.noFolder")}
      </button>
      <ul aria-label={t("organize.tags")} className="m-0 flex list-none flex-wrap gap-1.5 p-0">
        {row.tags.map((x) => (
          <li key={x.gid} className={chip}>
            <Icon name="label" size={14} />
            {x.name}
            <button
              type="button"
              aria-label={t("organize.removeTag", { name: x.name })}
              onClick={() => void actions.untag([meeting], x)}
              className="-mr-1 grid size-5 place-items-center rounded-full hover:bg-sunk"
            >
              <Icon name="close" size={12} />
            </button>
          </li>
        ))}
      </ul>
      <Popover open={adding} onOpenChange={setAdding} label={t("organize.addTag")} trigger={<button type="button" className={chip + " border-dashed hover:border-accent hover:text-accent"}><Icon name="add" size={14} />{t("organize.addTag")}</button>}>
        <TagCombobox tags={tags} exclude={have} autoFocus onChoose={(c) => void choose(c)} />
      </Popover>
      <MoveDialog open={moving} meetings={[meeting]} onClose={() => setMoving(false)} />
    </div>
  );
}
