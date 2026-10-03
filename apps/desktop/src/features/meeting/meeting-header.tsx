// SPDX-License-Identifier: Apache-2.0
// Detail header (D6): editable title, when and how long, source chips,
// participants, status and where the notes were written.
import {
  Icon,
  StatusPill,
  cn,
  useToast,
  usePlatform,
} from "@ghi/ui";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingDetail } from "../../bindings";
import { ipc } from "../../ipc";
import { meetingKeys, useMeetingNotes } from "../../state/meeting-queries";
import { durationLabel } from "../library/duration";
import { MEETINGS_KEY } from "../library/use-meetings";
import { detailStatus } from "./detail-status";
import { sourceAppName } from "../folders/source-app";
import { FolderTags } from "../folders/folder-tags";
import { StoredSpeaker } from "../speakers/stored-speaker";
import { useLlmName } from "./llm-name";
import { useWhen } from "./use-when";

function Chip({
  icon,
  children,
  dashed,
  tone,
}: {
  icon: Parameters<typeof Icon>[0]["name"];
  children: ReactNode;
  dashed?: boolean;
  tone?: "warn";
}) {
  return (
    <span
      className={cn(
        "inline-flex h-6 items-center gap-1 rounded-full border px-2 text-[12px]",
        dashed
          ? "border-dashed border-line2 text-muted"
          : tone === "warn"
            ? "border-transparent bg-warn-soft font-semibold text-warn"
            : "border-line2 text-muted",
      )}
    >
      <Icon name={icon} size={14} />
      {children}
    </span>
  );
}

function TitleField({ detail }: { detail: MeetingDetail }) {
  const { t } = useTranslation();
  const client = useQueryClient();
  const { show } = useToast();
  const [draft, setDraft] = useState(detail.title);
  const [seen, setSeen] = useState(detail.title);
  if (seen !== detail.title) {
    setSeen(detail.title);
    setDraft(detail.title);
  }
  const save = async () => {
    const title = draft.trim();
    if (!title) return setDraft(detail.title);
    if (title === detail.title) return;
    const key = meetingKeys.detail(detail.gid);
    client.setQueryData<MeetingDetail>(key, (d) => (d ? { ...d, title } : d));
    const r = await ipc.commands.setMeetingTitle(detail.gid, title);
    if (r.status === "error") {
      client.setQueryData<MeetingDetail>(key, (d) =>
        d ? { ...d, title: detail.title } : d,
      );
      setDraft(detail.title);
      show({
        tone: "warning",
        title: t("system.commandFailed", { message: r.error }),
      });
    } else void client.invalidateQueries({ queryKey: MEETINGS_KEY });
  };
  return (
    <input
      value={draft}
      aria-label={t("live.titleLabel")}
      placeholder={t("live.titlePlaceholder")}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={() => void save()}
      onKeyDown={(e) => {
        if (e.key === "Enter" && !e.nativeEvent.isComposing)
          e.currentTarget.blur();
        else if (e.key === "Escape") {
          setDraft(detail.title);
          e.currentTarget.blur();
        }
      }}
      className="text-title m-0 w-full min-w-0 truncate rounded-seg border-0 bg-transparent p-0 outline-none focus-visible:ring-2 focus-visible:ring-accent"
    />
  );
}

/** Where the notes came from: the local model, or the cloud when it was used. */
function EnginePill({ detail }: { detail: MeetingDetail }) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const installed = useLlmName();
  // The model that wrote these notes (recorded with them), else the one installed.
  const model = detail.notesModel ?? installed;
  const cloud = detail.cloudUsed;
  const notes = useMeetingNotes(detail.gid);
  // Nothing was written yet (still processing, or failed): no engine to name.
  if (!notes.data?.blocks.length) return null;
  return (
    <span
      className={cn(
        "inline-flex h-[26px] flex-none items-center gap-1.5 rounded-full px-2.5 text-[12px] font-semibold",
        cloud ? "bg-warn-soft text-warn" : "bg-sunk text-muted",
      )}
    >
      <Icon name={cloud ? "cloud" : "memory"} size={15} />
      {cloud
        ? t("library.status.cloudEnhanced")
        : model
          ? t("notes.engineLocal", { context: platform, model })
          : t("ask.onDevice", { context: platform })}
    </span>
  );
}

const SOURCE_ICON = { call: "videocam", room: "groups", import: "upload_file" } as const;

export function MeetingHeader({ detail }: { detail: MeetingDetail }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const when = useWhen();
  const { status, percent } = detailStatus(detail);
  const people = detail.speakers.filter((s) => !s.notPerson);
  const sourceKey = detail.source === "live" ? detail.mode : detail.source;
  const sourceLabel = ["call", "room", "mobile", "import"].includes(sourceKey)
    ? t(`library.sources.${sourceKey as "call"}`)
    : sourceKey;

  return (
    <header data-tauri-drag-region className="flex flex-col gap-1.5 px-7 pt-4 pb-2">
      <button
        type="button"
        onClick={() => void navigate({ to: "/meetings" })}
        className="flex items-center gap-0.5 self-start text-[12.5px] text-muted hover:text-ink"
      >
        <Icon name="chevron_left" size={16} />
        {t("detail.back")}
      </button>
      <div className="flex items-center gap-3.5">
        <div className="min-w-0 flex-1">
          <TitleField detail={detail} />
        </div>
        <EnginePill detail={detail} />
      </div>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 text-[13px] text-muted">
        <span className="flex items-center gap-1.5">
          <Icon name={SOURCE_ICON[sourceKey as keyof typeof SOURCE_ICON] ?? "headphones"} size={16} />
          {[
            detail.startedAt != null ? when(detail.startedAt) : null,
            detail.durationMs != null ? durationLabel(t, detail.durationMs) : null,
            sourceAppName(detail.sourceApp) ?? sourceLabel,
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
        {!detail.audioAvailable && (
          <Chip icon="link_off" dashed>
            {t("meeting.noAudio")}
          </Chip>
        )}
        {status !== "ready" && status !== "cloudEnhanced" && <StatusPill status={status} percent={percent} />}
      </div>
      <FolderTags meeting={detail.gid} />
      {people.length > 0 && (
        <ul aria-label={t("speakers.title")} className="m-0 flex list-none flex-wrap gap-1.5 p-0">
          {people.map((s) => (
            <li key={s.gid} className="flex h-7 items-center gap-1.5">
              <StoredSpeaker meeting={detail.gid} mode={detail.mode} speaker={s} />
            </li>
          ))}
        </ul>
      )}
    </header>
  );
}
