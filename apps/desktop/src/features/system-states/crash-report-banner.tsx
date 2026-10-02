// SPDX-License-Identifier: Apache-2.0
// Once per launch: the previous run ended without a clean exit. The report is
// already on this Mac (nothing is sent); the user can open its folder.
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { APP_NAME } from "@ghi/i18n";
import { Button } from "@ghi/ui";
import { ipc } from "../../ipc";
import { SystemBanner } from "./system-banner";

export function CrashReportBanner() {
  const { t } = useTranslation();
  const [gone, setGone] = useState(false);
  // Once per launch: acknowledging clears it in the core, so never refetch.
  const { data } = useQuery({
    queryKey: ["diagnostics-status"],
    queryFn: () => ipc.commands.diagnosticsStatus(),
    staleTime: Infinity,
    gcTime: Infinity,
    refetchOnWindowFocus: false,
  });
  if (gone || !data?.crashedLastRun) return null;

  const dismiss = () => {
    setGone(true);
    void ipc.commands.acknowledgeCrash();
  };

  return (
    <SystemBanner
      id="crash-report"
      icon="warning"
      title={t("system.crashReport.title", { app: APP_NAME })}
      onDismiss={dismiss}
      actions={
        <Button
          size="sm"
          variant="primary"
          onClick={() => {
            void ipc.commands.revealDiagnostics();
            dismiss();
          }}
        >
          {t("system.crashReport.reveal")}
        </Button>
      }
    >
      {t("system.crashReport.body")}
    </SystemBanner>
  );
}
