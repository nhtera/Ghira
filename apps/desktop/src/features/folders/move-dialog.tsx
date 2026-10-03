// SPDX-License-Identifier: Apache-2.0
// "Move to folder…": pick a folder, "No folder", or type a new one.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Dialog, Icon } from "@ghi/ui";
import type { FolderRow } from "../../bindings";
import { MAX, useFolders, useOrganizeActions } from "./organize";

function Body({ meetings, onClose, onMoved }: { meetings: string[]; onClose: () => void; onMoved?: () => void }) {
  const { t } = useTranslation();
  const folders = useFolders().data ?? [];
  const actions = useOrganizeActions();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const move = async (folder: FolderRow | null) => {
    if (busy) return;
    setBusy(true);
    const ok = await actions.move(meetings, folder);
    setBusy(false);
    if (ok) {
      onMoved?.();
      onClose();
    }
  };
  const create = async () => {
    const value = name.trim();
    if (!value || busy) return;
    setBusy(true);
    const folder = await actions.createFolder(value);
    setBusy(false);
    if (folder) await move(folder);
  };
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()} title={t("organize.moveTo")} width={400} footer={<Button onClick={onClose}>{t("common.cancel")}</Button>}>
      <ul className="m-0 flex max-h-64 list-none flex-col gap-0.5 overflow-auto p-0">
        <li>
          <button type="button" onClick={() => void move(null)} className="flex h-9 w-full items-center gap-2 rounded-seg px-2 text-left text-[13px] hover:bg-sunk">
            <Icon name="folder" size={18} className="text-muted" />
            {t("organize.noFolder")}
          </button>
        </li>
        {folders.map((f) => (
          <li key={f.gid}>
            <button type="button" onClick={() => void move(f)} className="flex h-9 w-full items-center gap-2 rounded-seg px-2 text-left text-[13px] hover:bg-sunk">
              <Icon name="folder" size={18} className="text-muted" />
              <span className="min-w-0 flex-1 truncate">{f.name}</span>
              <span className="text-small text-muted">{f.meetings}</span>
            </button>
          </li>
        ))}
      </ul>
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
      >
        <input
          value={name}
          maxLength={MAX.folderName * 2}
          onChange={(e) => setName(e.target.value)}
          placeholder={t("organize.newFolder")}
          aria-label={t("organize.folderName")}
          className="text-body h-8 min-w-0 flex-1 rounded-ctl border border-ctl bg-surface px-2.5 text-ink"
        />
        <Button type="submit" icon="add" disabled={!name.trim() || busy} aria-label={t("organize.newFolder")} />
      </form>
    </Dialog>
  );
}

export function MoveDialog({ open, meetings, onClose, onMoved }: { open: boolean; meetings: string[]; onClose: () => void; onMoved?: () => void }) {
  // Mounted only while open: each opening starts with an empty field.
  return open ? <Body meetings={meetings} onClose={onClose} onMoved={onMoved} /> : null;
}
