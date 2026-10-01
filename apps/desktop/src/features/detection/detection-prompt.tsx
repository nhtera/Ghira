// SPDX-License-Identifier: Apache-2.0
// D2, in-window version: a small card at the top right when a meeting app
// starts using the mic. Not a modal and never takes focus (no autofocus), so
// typing or recording is not interrupted. The native panel (main window
// hidden) is `DetectPanel`; both draw the same `DetectionCard`.
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, cn, useToast } from "@ghi/ui";
import type { DetectReply, MeetingDetected } from "../../bindings";
import { ipc } from "../../ipc";
import { useAppActions } from "../../shell/actions";

/** The prompt itself: what was detected and the three answers. */
export function DetectionCard({
  detected,
  onReply,
  className,
}: {
  detected: MeetingDetected;
  onReply: (r: DetectReply) => void;
  className?: string;
}) {
  const { t } = useTranslation();
  // Browsers prompt generically; the core sends a display name for them.
  const meetingApp = detected.appName;
  return (
    <div
      role="region"
      aria-label={t("tray.detected.title", { meetingApp })}
      className={cn(
        "flex flex-col gap-3 rounded-panel border border-line2 bg-surface p-4",
        className,
      )}
    >
      <div className="flex items-start gap-2.5">
        <Icon
          name="videocam"
          size={20}
          className="mt-px flex-none text-accent"
        />
        <p className="m-0 text-[13.5px] leading-snug font-semibold text-ink">
          {t("tray.detected.title", { meetingApp })}
        </p>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="primary" onClick={() => onReply("start")}>
          {t("tray.detected.start")}
        </Button>
        <Button size="sm" onClick={() => onReply("notNow")}>
          {t("tray.detected.notNow")}
        </Button>
        <Button size="sm" variant="ghost" onClick={() => onReply("never")}>
          {t("tray.detected.never", { meetingApp })}
        </Button>
      </div>
    </div>
  );
}

export function DetectionPrompt() {
  const { t } = useTranslation();
  const { show } = useToast();
  const { startRecording } = useAppActions();
  const [detected, setDetected] = useState<MeetingDetected | null>(null);

  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void ipc
      .onMeetingDetected(setDetected)
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  const reply = useCallback(
    async (d: MeetingDetected, r: DetectReply) => {
      setDetected(null);
      if (r === "start") {
        const started = await startRecording("call");
        if (started?.status === "error") return;
      }
      const res = await ipc.commands.replyMeetingDetected(d.app, r);
      if (res.status === "error")
        show({
          tone: "warning",
          title: t("system.commandFailed", { message: res.error }),
        });
    },
    [startRecording, show, t],
  );

  const meetingApp = detected?.appName ?? "";
  return (
    <>
      {/* Always mounted so screen readers pick up the change; the card never takes focus. */}
      <div role="status" aria-live="polite" className="sr-only">
        {detected ? t("tray.detected.title", { meetingApp }) : ""}
      </div>
      {detected && (
        <DetectionCard
          detected={detected}
          onReply={(r) => void reply(detected, r)}
          className="fixed top-14 right-4 z-40 w-80 max-w-[calc(100vw-2rem)] shadow-float"
        />
      )}
    </>
  );
}
