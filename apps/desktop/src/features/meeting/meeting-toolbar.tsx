// SPDX-License-Identifier: Apache-2.0
// The detail's tab row (D6): the tabs, then My notes only, the notes language,
// Improve with cloud and one Export menu that also holds Regenerate (after
// asking: what you wrote, edited, pinned or ticked stays), the template, Ask,
// the follow-up email and the never-cloud switch.
import {
  Icon,
  InlineConfirm,
  Menu,
  cn,
  useToast,
  type MenuItem,
} from "@ghi/ui";
import { useQueryClient } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingDetail } from "../../bindings";
import { FollowupEmailDialog } from "../email/followup-email-dialog";
import { ipc } from "../../ipc";
import { invalidateMeeting, useTemplates } from "../../state/meeting-queries";
import { useCloudOffered } from "../cloud-sheet/cloud-offered";
import { SensitiveConfirm } from "../sensitive";
import { useNotesLanguage, type PickedLanguage } from "./notes-language";
import { DEFAULT_TEMPLATE, templateName } from "./template-names";
import { inProgress } from "../library/meeting-status";

const LANGS = ["en", "vi"] as const;

export type MeetingToolbarProps = {
  detail: MeetingDetail;
  /** The tab buttons, at the left of the row. */
  tabs: ReactNode;
  /** My notes only belongs to the Notes tab: `undefined` hides the switch. */
  onlyMine?: boolean;
  onOnlyMine?: (on: boolean) => void;
  onExport: () => void;
  onImproveWithCloud?: () => void;
  onAsk?: () => void;
};

export function MeetingToolbar({
  detail,
  tabs,
  onlyMine,
  onOnlyMine,
  onExport,
  onImproveWithCloud,
  onAsk,
}: MeetingToolbarProps) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const { show } = useToast();
  const templates = useTemplates();
  const cloudOffered = useCloudOffered();
  const current = detail.template ?? DEFAULT_TEMPLATE;
  const [template, setTemplate] = useState(current);
  const [seenCurrent, setSeenCurrent] = useState(current);
  if (seenCurrent !== current) {
    // The notes were regenerated with another template: follow it.
    setSeenCurrent(current);
    setTemplate(current);
  }
  const [picked, setPicked] = useState<PickedLanguage | null>(null);
  const { shown, request: language } = useNotesLanguage(detail, picked);
  const [asking, setAsking] = useState(false);
  const [sensitiveAsk, setSensitiveAsk] = useState(false);
  const [emailing, setEmailing] = useState(false);
  const [busy, setBusy] = useState(false);
  // Choosing a language or template only stages it: ask right away what Regenerate will do.
  const choose = (apply: () => void) => {
    apply();
    if (!busy && !inProgress(detail.status) && detail.job == null) setAsking(true);
  };
  const languageName = language === "en" ? t("import.options.languages.english") : language === "vi" ? t("import.options.languages.vietnamese") : t("meeting.lang.meeting");
  const busyJob = busy || inProgress(detail.status) || detail.job != null;
  const failed = (message: string) =>
    show({ tone: "warning", title: t("system.commandFailed", { message }) });

  const regenerate = async () => {
    setAsking(false);
    setBusy(true);
    const r = await ipc.commands.regenerateNotes(
      detail.gid,
      template === current ? null : template,
      language,
    );
    setBusy(false);
    if (r.status === "error") return failed(r.error);
    // true: written now; false: queued until the speech models are installed.
    show({
      tone: "info",
      title: r.data ? t("meeting.regenerating") : t("meeting.regenWaiting"),
    });
    void invalidateMeeting(client, detail.gid);
  };

  const copyMarkdown = async () => {
    const r = await ipc.commands.meetingAsText(detail.gid, true, {
      notes: true,
      transcript: false,
      vietnamese: i18n.language === "vi",
    });
    if (r.status === "error") return failed(r.error);
    try {
      await navigator.clipboard.writeText(r.data);
      show({ tone: "success", title: t("library.notesCopied") });
    } catch (e) {
      failed(String(e));
    }
  };

  const templateItems: MenuItem[] = (templates.data ?? []).map((tpl) => ({
    label: templateName(tpl.id, t, tpl.name),
    icon: tpl.id === template ? "check" : undefined,
    onSelect: () => choose(() => setTemplate(tpl.id)),
  }));
  const toggleCloudLock = async () => {
    const r = await ipc.commands.setMeetingCloudLocked(
      detail.gid,
      !detail.cloudLocked,
    );
    if (r.status === "error") return failed(r.error);
    void invalidateMeeting(client, detail.gid);
  };
  // On asks first (the audio is deleted now); off needs no question.
  const toggleSensitive = async () => {
    if (!detail.sensitive) return setSensitiveAsk(true);
    const r = await ipc.commands.setMeetingSensitive(detail.gid, false);
    if (r.status === "error") return failed(r.error);
    show({ tone: "info", title: t("sensitive.turnedOff") });
    void invalidateMeeting(client, detail.gid);
  };
  const exportItems: MenuItem[] = [
    {
      label: t("meeting.exportEllipsis"),
      icon: "ios_share",
      onSelect: onExport,
    },
    {
      label: t("detail.more.copyMarkdown"),
      icon: "content_copy",
      onSelect: () => void copyMarkdown(),
    },
    {
      label: t("detail.more.draftEmail"),
      icon: "inbox",
      onSelect: () => setEmailing(true),
    },
    ...(onAsk ? [{ label: t("ask.meeting.open"), icon: "forum", onSelect: onAsk, movesFocus: true } satisfies MenuItem] : []),
    { kind: "separator" },
    {
      label: t("detail.more.regenerate"),
      icon: "refresh",
      disabled: busyJob || asking,
      onSelect: () => setAsking(true),
    },
    ...templateItems,
    { kind: "separator" },
    {
      label: t("cloud.sheet.never"),
      icon: detail.cloudLocked ? "check" : "cloud_off",
      onSelect: () => void toggleCloudLock(),
    },
    {
      label: t("sensitive.menu"),
      icon: detail.sensitive ? "check" : "visibility_off",
      movesFocus: !detail.sensitive,
      onSelect: () => void toggleSensitive(),
    },
  ];
  const ctl = "h-[30px] rounded-ctl border border-ctl bg-surface px-[11px] text-[12.5px] font-medium";

  return (
    <div className="border-b border-line px-7 pt-2.5">
      <div className="relative flex flex-wrap items-end gap-x-2.5">
        {tabs}
        <span className="flex-1" />
        <div className="flex flex-wrap items-center gap-2 pb-1.5">
          {onOnlyMine && (
            <button
              type="button"
              role="switch"
              aria-checked={!!onlyMine}
              onClick={() => onOnlyMine(!onlyMine)}
              className="flex h-[30px] items-center gap-2 text-[12.5px] text-muted"
            >
              <span className={cn("relative h-[18px] w-8 rounded-[9px] transition-colors", onlyMine ? "bg-accent" : "bg-line2")}>
                <i className={cn("absolute top-0.5 size-3.5 rounded-full bg-white transition-[left]", onlyMine ? "left-4" : "left-0.5")} />
              </span>
              {t("notes.onlyMine")}
            </button>
          )}
          <div role="group" aria-label={t("common.language")} className="flex gap-0.5 rounded-ctl border border-ctl p-0.5">
            {LANGS.map((l) => (
              <button
                key={l}
                type="button"
                aria-pressed={shown === l}
                title={l === "en" ? t("import.options.languages.english") : t("import.options.languages.vietnamese")}
                // Any click sets the language explicitly.
                onClick={() => picked !== l && choose(() => setPicked(l))}
                className={cn("h-6 rounded-seg px-[9px] text-[12px] font-semibold", shown === l ? "bg-accent-soft text-accent" : "text-muted hover:text-ink")}
              >
                {l.toUpperCase()}
              </button>
            ))}
          </div>
          {cloudOffered && !detail.sensitive && (
            <button type="button" onClick={onImproveWithCloud} className={cn(ctl, "inline-flex items-center gap-[5px] hover:bg-surface2")}>
              <Icon name="cloud_upload" size={16} />
              {t("notes.improveWithCloud")}
            </button>
          )}
          <Menu
            label={t("common.export")}
            items={exportItems}
            trigger={
              <button type="button" className={cn(ctl, "inline-flex items-center gap-[5px] hover:bg-surface2")}>
                <Icon name="ios_share" size={16} />
                {t("common.export")}
                <Icon name="expand_more" size={16} />
              </button>
            }
          />
        </div>
      </div>
      {asking && (
        <div className="pb-2">
          <InlineConfirm
            icon="refresh"
            question={`${t("meeting.regenerateWith", { template: templateName(template, t, templates.data?.find((x) => x.id === template)?.name), language: languageName })} ${t("meeting.regenerateQuestion")}`}
            confirmLabel={t("detail.more.regenerate")}
            onConfirm={() => void regenerate()}
            onCancel={() => setAsking(false)}
          />
        </div>
      )}
      {sensitiveAsk && (
        <div className="pb-2">
          <SensitiveConfirm meeting={detail.gid} recording={false} onClose={() => setSensitiveAsk(false)} />
        </div>
      )}
      <FollowupEmailDialog
        open={emailing}
        onOpenChange={setEmailing}
        meeting={detail.gid}
      />
    </div>
  );
}
