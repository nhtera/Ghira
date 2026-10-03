// SPDX-License-Identifier: Apache-2.0
// Sidebar folder list under Meetings, with New folder… (phase 14d). A folder
// opens the library filtered to it (`/meetings?folder=<gid>`).
import { Link, useSearch } from "@tanstack/react-router";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Icon, cn } from "@ghi/ui";
import { MAX, useFolders, useOrganizeActions } from "./organize";

const item =
  "flex h-8 items-center gap-2.5 rounded-ctl px-2.5 text-[13px] font-medium text-muted no-underline hover:bg-sunk " +
  "aria-[current=true]:bg-surface aria-[current=true]:text-ink aria-[current=true]:shadow-[0_0_0_1px_var(--line)]";

export function SidebarFolders() {
  const { t } = useTranslation();
  const folders = useFolders().data ?? [];
  const actions = useOrganizeActions();
  const current = (useSearch({ strict: false }) as { folder?: string }).folder;
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  // A second Enter while the first is in flight must not make the folder twice.
  const busy = useRef(false);
  const create = async () => {
    const value = name.trim();
    if (!value) return setAdding(false);
    if (busy.current) return;
    busy.current = true;
    const folder = await actions.createFolder(value).finally(() => (busy.current = false));
    if (folder) {
      setName("");
      setAdding(false);
    }
  };
  return (
    <section aria-label={t("organize.folders")} className="mt-3 flex flex-col gap-0.5">
      <h2 className="text-small m-0 px-2.5 pb-0.5 font-semibold text-muted">{t("organize.folders")}</h2>
      {folders.map((f) => (
        <Link key={f.gid} to="/meetings" search={{ folder: f.gid }} aria-current={current === f.gid} className={item}>
          <Icon name="folder" size={18} />
          <span className="min-w-0 flex-1 truncate">{f.name}</span>
          <span className="text-small font-normal text-faint">{f.meetings}</span>
        </Link>
      ))}
      {adding ? (
        <input
          autoFocus
          value={name}
          maxLength={MAX.folderName * 2}
          aria-label={t("organize.folderName")}
          placeholder={t("organize.folderName")}
          onChange={(e) => setName(e.target.value)}
          onBlur={() => !name.trim() && setAdding(false)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              setName("");
              setAdding(false);
            } else if (e.key === "Enter" && !e.nativeEvent.isComposing && e.keyCode !== 229) {
              e.preventDefault();
              void create();
            }
          }}
          className="text-body mx-1 h-8 rounded-ctl border border-ctl bg-surface px-2.5 text-ink"
        />
      ) : (
        <button type="button" onClick={() => setAdding(true)} className={cn(item, "text-left")}>
          <Icon name="add" size={18} />
          {t("organize.newFolder")}
        </button>
      )}
    </section>
  );
}
