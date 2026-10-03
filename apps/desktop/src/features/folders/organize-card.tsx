// SPDX-License-Identifier: Apache-2.0
// Settings → General "Folders and tags": rename and delete with counts, the
// delete behind an inline confirm (the meetings stay either way).
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, ConfirmArea, useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { Card } from "../settings/parts";
import { MAX, organizeError, useFolders, useInvalidateOrganize, useTags } from "./organize";

type Item = { gid: string; name: string; meetings: number };

function ItemRow({ item, kind }: { item: Item; kind: "folder" | "tag" }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const invalidate = useInvalidateOrganize();
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(item.name);
  const max = kind === "folder" ? MAX.folderName : MAX.tagName;
  const fail = (code: string, shown?: string) => {
    const title = organizeError(t, code, { name: shown, max });
    if (title) show({ tone: "warning", title });
  };
  const rename = async () => {
    const value = name.trim();
    if (!value || value === item.name) return setEditing(false);
    const r = kind === "folder" ? await ipc.commands.renameFolder(item.gid, value) : await ipc.commands.renameTag(item.gid, value);
    if (r.status === "error") return fail(r.error, value);
    setEditing(false);
    invalidate();
  };
  const remove = async () => {
    const r = kind === "folder" ? await ipc.commands.deleteFolder(item.gid) : await ipc.commands.deleteTag(item.gid);
    if (r.status === "error") return fail(r.error);
    invalidate();
  };
  const question = kind === "folder" ? t("organize.deleteFolderQuestion", { name: item.name, count: item.meetings }) : t("organize.deleteTagQuestion", { name: item.name, count: item.meetings });
  const label = kind === "folder" ? t("organize.renameFolder") : t("organize.renameTag");
  return (
    <li className="flex flex-col gap-2 border-t border-line py-2 first:border-t-0">
      <div className="flex items-center gap-2">
        {editing ? (
          <form
            className="flex min-w-0 flex-1 gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              void rename();
            }}
          >
            <input
              autoFocus
              value={name}
              maxLength={max * 2}
              aria-label={label}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => e.key === "Escape" && (e.stopPropagation(), setEditing(false), setName(item.name))}
              className="text-body h-8 min-w-0 flex-1 rounded-ctl border border-ctl bg-surface px-2.5 text-ink"
            />
            <Button type="submit" variant="primary" disabled={!name.trim()}>
              {t("common.save")}
            </Button>
          </form>
        ) : (
          <>
            <span className="min-w-0 flex-1 truncate text-[13.5px]">{item.name}</span>
            <span className="text-small text-muted">{t("people.meetingsCount", { count: item.meetings })}</span>
            <Button size="sm" aria-label={`${label}: ${item.name}`} onClick={() => setEditing(true)}>
              {label}
            </Button>
          </>
        )}
        {!editing && (
          <ConfirmArea
            icon="delete_forever"
            question={question}
            confirmLabel={t("common.delete")}
            onConfirm={() => void remove()}
            trigger={({ onClick, ref }) => (
              <Button ref={ref} size="sm" icon="delete" className="border-rec text-rec" aria-label={`${kind === "folder" ? t("organize.deleteFolder") : t("organize.deleteTag")}: ${item.name}`} onClick={onClick} />
            )}
          />
        )}
      </div>
    </li>
  );
}

export function OrganizeCard() {
  const { t } = useTranslation();
  const folders = useFolders().data ?? [];
  const tags = useTags().data ?? [];
  return (
    <Card title={t("organize.manageTitle")}>
      {folders.length + tags.length === 0 && <p className="text-small m-0 text-muted">{t("organize.manageEmpty")}</p>}
      {folders.length > 0 && (
        <section aria-label={t("organize.folders")}>
          <h4 className="text-small m-0 mb-1 font-semibold text-muted">{t("organize.folders")}</h4>
          <ul className="m-0 list-none p-0">
            {folders.map((f) => (
              <ItemRow key={f.gid} item={f} kind="folder" />
            ))}
          </ul>
        </section>
      )}
      {tags.length > 0 && (
        <section aria-label={t("organize.tags")}>
          <h4 className="text-small m-0 mb-1 font-semibold text-muted">{t("organize.tags")}</h4>
          <ul className="m-0 list-none p-0">
            {tags.map((x) => (
              <ItemRow key={x.gid} item={x} kind="tag" />
            ))}
          </ul>
        </section>
      )}
    </Card>
  );
}
