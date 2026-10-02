// SPDX-License-Identifier: Apache-2.0
// Detail header (D6): editable title, when and how long, source chips,
// participants, status and where the notes were written.
import { formatClock, formatDate, formatTime, type Locale } from "@ghi/i18n";
import {
  Icon,
  SpeakerChip,
  StatusPill,
  cn,
  useToast,
  usePlatform,
} from "@ghi/ui";
import { useQueryClient } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingDetail } from "../../bindings";
import { ipc } from "../../ipc";
import { meetingKeys } from "../../state/meeting-queries";
import { MEETINGS_KEY } from "../library/use-meetings";
import { detailStatus } from "./detail-status";
import { speakerDisplay } from "./speaker-display";

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

export function MeetingHeader({ detail }: { detail: MeetingDetail }) {
  const { t, i18n } = useTranslation();
  const platform = usePlatform();
  const locale: Locale = i18n.language === "vi" ? "vi" : "en";
  const { status, percent } = detailStatus(detail);
  const people = detail.speakers.filter((s) => !s.notPerson);
  const sourceKey = detail.source === "live" ? detail.mode : detail.source;
  const sourceLabel = ["call", "room", "mobile", "import"].includes(sourceKey)
    ? t(`library.sources.${sourceKey as "call"}`)
    : sourceKey;

  return (
    <header
      data-tauri-drag-region
      className="flex flex-col gap-2 px-7 pt-6 pb-2"
    >
      <TitleField detail={detail} />
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5 text-[12.5px] text-muted">
        {detail.startedAt != null && (
          <span>
            {formatDate(detail.startedAt, locale)} ·{" "}
            {formatTime(detail.startedAt, locale)}
          </span>
        )}
        {detail.durationMs != null && (
          <span className="text-mono">{formatClock(detail.durationMs)}</span>
        )}
        <Chip
          icon={sourceKey === "room" ? "groups" : "headphones"}
          dashed={!detail.audioAvailable}
        >
          {sourceLabel}
        </Chip>
        {!detail.audioAvailable && (
          <Chip icon="link_off" dashed>
            {t("meeting.noAudio")}
          </Chip>
        )}
        <StatusPill status={status} percent={percent} />
        {detail.cloudUsed ? (
          // The pill already says it when nothing else outranks it.
          status !== "cloudEnhanced" && (
            <Chip icon="cloud_done" tone="warn">
              {t("library.status.cloudEnhanced")}
            </Chip>
          )
        ) : (
          <Chip icon="lock">{t("cloud.before", { context: platform })}</Chip>
        )}
      </div>
      {people.length > 0 && (
        <ul
          aria-label={t("speakers.title")}
          className="m-0 flex list-none flex-wrap gap-1.5 p-0"
        >
          {people.map((s) => {
            const d = speakerDisplay(s, t);
            return (
              <li key={s.gid} className="h-7">
                <SpeakerChip
                  state={d.named ? "named" : "numbered"}
                  name={d.name}
                  colorSlot={d.colorSlot}
                  isMe={d.isMe}
                />
              </li>
            );
          })}
        </ul>
      )}
    </header>
  );
}
