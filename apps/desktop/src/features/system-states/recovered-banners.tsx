// SPDX-License-Identifier: Apache-2.0
// "Closed unexpectedly": the meetings crash recovery closed at this launch
// (the core hands them over once). The audio is safe; the user opens it or
// discards it (with a confirmation: it deletes the meeting).
import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { formatClock } from "@ghi/i18n";
import { Button, ConfirmArea, useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { SystemBanner } from "./system-banner";

const KEY = ["recovered-meetings"] as const;

export function RecoveredBanners() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { show } = useToast();
  const queryClient = useQueryClient();
  const [gone, setGone] = useState<string[]>([]);
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
  const hide = (gid: string) => setGone((g) => [...g, gid]);

  const discard = async (gid: string) => {
    const r = await ipc.commands.deleteMeeting(gid);
    if (r.status === "error") return show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
    hide(gid);
    void queryClient.invalidateQueries({ queryKey: ["meetings"] });
  };

  return (
    <>
      {(data ?? [])
        .filter((m) => !gone.includes(m.gid))
        .map((m) => (
          <SystemBanner
            key={m.gid}
            id="recovered"
            icon="warning"
            title={t("system.crash.title")}
            onDismiss={() => hide(m.gid)}
            actions={
              <>
                <Button size="sm" variant="primary" onClick={() => {
                    hide(m.gid);
                    void navigate({ to: "/meetings/$id/$tab", params: { id: m.gid, tab: "notes" } });
                  }}>
                  {t("system.crash.open")}
                </Button>
                <ConfirmArea
                  question={t("system.crash.discardQuestion")}
                  confirmLabel={t("system.crash.discard")}
                  onConfirm={() => void discard(m.gid)}
                  trigger={({ onClick, ref }) => (
                    <Button ref={ref} size="sm" onClick={onClick}>
                      {t("system.crash.discard")}
                    </Button>
                  )}
                />
              </>
            }
          >
            {t("system.crash.body", { title: m.title, time: formatClock(m.durationMs ?? 0) })}
          </SystemBanner>
        ))}
    </>
  );
}
