// SPDX-License-Identifier: Apache-2.0
// "Closed unexpectedly": the meetings crash recovery closed at this launch
// (the core hands them over once). The audio is safe and its notes are already
// queued; the user opens the meeting or discards it (with a confirmation: it
// deletes the meeting). One at a time; Escape leaves the rest for later.
import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { formatClock } from "@ghi/i18n";
import { Button, Dialog, Icon, useToast } from "@ghi/ui";
import { ipc } from "../../ipc";

const KEY = ["recovered-meetings"] as const;

export function RecoveredDialog() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { show } = useToast();
  const queryClient = useQueryClient();
  const [gone, setGone] = useState<string[]>([]);
  const [asking, setAsking] = useState(false);
  // Once per launch: the query must never refetch (a second call returns nothing).
  const { data } = useQuery({
    queryKey: KEY,
    queryFn: async () => {
      const r = await ipc.commands.takeRecoveredMeetings();
      return r.status === "ok" ? r.data : [];
    },
    staleTime: Infinity,
    gcTime: Infinity,
    refetchOnWindowFocus: false,
  });
  const hide = (gid: string) => {
    setAsking(false);
    setGone((g) => [...g, gid]);
  };

  const discard = async (gid: string) => {
    const r = await ipc.commands.deleteMeeting(gid);
    if (r.status === "error") return show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
    hide(gid);
    void queryClient.invalidateQueries({ queryKey: ["meetings"] });
  };

  const m = (data ?? []).find((x) => !gone.includes(x.gid));
  if (!m) return null;
  return (
    <Dialog
      open
      placement="center"
      width={440}
      onOpenChange={(o) => !o && hide(m.gid)}
      title={
        <span className="flex flex-col items-start gap-2.5">
          <Icon name="update" size={30} className="text-accent" />
          <span className="text-[17px] font-semibold">{t("system.crash.title")}</span>
        </span>
      }
      description={t("system.crash.body", { title: m.title, time: formatClock(m.durationMs ?? 0) })}
      footer={
        asking ? (
          <>
            <Button onClick={() => setAsking(false)}>{t("common.cancel")}</Button>
            <Button variant="danger" onClick={() => void discard(m.gid)}>
              {t("system.crash.discard")}
            </Button>
          </>
        ) : (
          // The way forward first, as in the design; the destructive choice is quiet and apart.
          <div className="flex w-full items-center justify-between">
            <Button
              variant="primary"
              size="lg"
              autoFocus
              onClick={() => {
                hide(m.gid);
                void navigate({ to: "/meetings/$id/$tab", params: { id: m.gid, tab: "notes" } });
              }}
            >
              {t("system.crash.recover")}
            </Button>
            <Button variant="ghost" size="lg" className="text-rec-ink" onClick={() => setAsking(true)}>
              {t("system.crash.discard")}
            </Button>
          </div>
        )
      }
    >
      {asking && (
        <p role="alert" className="text-body m-0 font-semibold text-rec-ink">
          {t("system.crash.discardQuestion")}
        </p>
      )}
    </Dialog>
  );
}
