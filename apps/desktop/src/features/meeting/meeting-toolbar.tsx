// SPDX-License-Identifier: Apache-2.0
// Detail toolbar (D6): template, notes language, Regenerate (after asking:
// what you wrote, edited, pinned or ticked stays) and Share / Export.
import {
  Button,
  Icon,
  InlineConfirm,
  Menu,
  Segmented,
  useToast,
  type MenuItem,
} from "@ghi/ui";
import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingDetail, NotesLanguage } from "../../bindings";
import { FollowupEmailDialog } from "../email/followup-email-dialog";
import { ipc } from "../../ipc";
import { invalidateMeeting, useTemplates } from "../../state/meeting-queries";
import { DEFAULT_TEMPLATE, templateName } from "./template-names";
import { inProgress } from "../library/meeting-status";

export type MeetingToolbarProps = {
  detail: MeetingDetail;
  onExport: () => void;
  onImproveWithCloud?: () => void;
  onAsk?: () => void;
};

export function MeetingToolbar({
  detail,
  onExport,
  onImproveWithCloud,
  onAsk,
}: MeetingToolbarProps) {
  const { t, i18n } = useTranslation();
  const client = useQueryClient();
  const { show } = useToast();
  const templates = useTemplates();
  const current = detail.template ?? DEFAULT_TEMPLATE;
  const [template, setTemplate] = useState(current);
  const [seenCurrent, setSeenCurrent] = useState(current);
  if (seenCurrent !== current) {
    // The notes were regenerated with another template: follow it.
    setSeenCurrent(current);
    setTemplate(current);
  }
  const [language, setLanguage] = useState<NotesLanguage>("meeting");
  const [asking, setAsking] = useState(false);
  const [emailing, setEmailing] = useState(false);
  const [busy, setBusy] = useState(false);
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
    onSelect: () => setTemplate(tpl.id),
  }));
  const toggleCloudLock = async () => {
    const r = await ipc.commands.setMeetingCloudLocked(
      detail.gid,
      !detail.cloudLocked,
    );
    if (r.status === "error") return failed(r.error);
    void invalidateMeeting(client, detail.gid);
  };
  const shareItems: MenuItem[] = [
    {
      label: t("detail.more.copyMarkdown"),
      icon: "content_copy",
      onSelect: () => void copyMarkdown(),
    },
    {
      label: t("meeting.exportEllipsis"),
      icon: "ios_share",
      onSelect: onExport,
    },
    {
      label: t("detail.more.draftEmail"),
      icon: "inbox",
      onSelect: () => setEmailing(true),
    },
    {
      label: t("cloud.sheet.never"),
      icon: detail.cloudLocked ? "check" : "cloud_off",
      onSelect: () => void toggleCloudLock(),
    },
  ];

  return (
    <div className="flex flex-col gap-2 px-7 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <Menu
          align="start"
          label={t("library.filters.template")}
          items={templateItems}
          trigger={
            <Button
              size="sm"
              icon="description"
              end={<Icon name="expand_more" size={16} />}
              aria-label={t("meeting.templateButton", {
                name: templateName(template, t),
              })}
            >
              {templateName(
                template,
                t,
                templates.data?.find((x) => x.id === template)?.name,
              )}
            </Button>
          }
        />
        <Segmented<NotesLanguage>
          label={t("common.language")}
          value={language}
          onChange={setLanguage}
          options={[
            { value: "meeting", label: t("meeting.lang.meeting") },
            { value: "en", label: t("import.options.languages.english") },
            { value: "vi", label: t("import.options.languages.vietnamese") },
          ]}
        />
        <Button
          size="sm"
          icon="refresh"
          disabled={busyJob || asking}
          onClick={() => setAsking(true)}
        >
          {t("detail.more.regenerate")}
        </Button>
        <Button size="sm" icon="cloud" onClick={onImproveWithCloud}>
          {t("notes.improveWithCloud")}
        </Button>
        {onAsk && (
          <Button size="sm" icon="forum" onClick={onAsk}>
            {t("ask.meeting.open")}
          </Button>
        )}
        <span className="flex-1" />
        <Menu
          label={t("meeting.share")}
          items={shareItems}
          trigger={
            <Button size="sm" icon="ios_share">
              {t("meeting.share")}
            </Button>
          }
        />
      </div>
      {asking && (
        <InlineConfirm
          icon="refresh"
          question={t("meeting.regenerateQuestion")}
          confirmLabel={t("detail.more.regenerate")}
          onConfirm={() => void regenerate()}
          onCancel={() => setAsking(false)}
        />
      )}
      <FollowupEmailDialog
        open={emailing}
        onOpenChange={setEmailing}
        meeting={detail.gid}
      />
    </div>
  );
}
